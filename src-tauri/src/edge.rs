//! 新版边缘加密通道 dx150-edge-v1。
//!
//! 站点 2026-11-12 起在页面里启用了加密接口：先 ECDH 握手拿会话密钥，
//! 再用 AES-256-GCM 收发信封。旧明文 /api/edge/request 目前仍可用，
//! 所以这里只做兜底：明文桥一旦消失（404/405/410）自动切过来，
//! 也可用环境变量 HGHUB_EDGE=encrypted 强制走这里。

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64U};
use base64::Engine as _;
use hkdf::Hkdf;
use once_cell::sync::Lazy;
use p256::ecdh::EphemeralSecret;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::PublicKey;
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const PROTOCOL: &str = "dx150-edge-v1";
const MAX_SEQ: u32 = 16384;

#[derive(Clone)]
struct Session {
    info: String,
    kid: String,
    sid: String,
    seq: u32,
    offset: i64,
    expires: i64,
    req: Aes256Gcm,
    res: Aes256Gcm,
}

static SESSIONS: Lazy<Mutex<HashMap<&'static str, Session>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static ROUTES: Lazy<Mutex<Option<Value>>> = Lazy::new(|| Mutex::new(None));

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn b64u(bytes: &[u8]) -> String {
    B64U.encode(bytes)
}

/// 响应体内的 base64（标准表，是否补齐都接受）。
fn unb64_std(s: &str) -> Result<Vec<u8>, String> {
    let t = s.trim_end_matches('=');
    B64.decode(s)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(t))
        .map_err(|_| "加密响应编码无效".to_string())
}

/// 握手/密文用的 base64url（无补齐）。
fn unb64_url(s: &str) -> Result<Vec<u8>, String> {
    B64U.decode(s.trim_end_matches('=')).map_err(|_| "加密响应编码无效".to_string())
}

/// HGHUB_EDGE=encrypted 时强制走加密通道（用于验证）。
pub fn forced() -> bool {
    std::env::var("HGHUB_EDGE").map(|v| v.eq_ignore_ascii_case("encrypted")).unwrap_or(false)
}

/// 明文桥被摘掉的特征（业务错误不在此列，不走兜底）。
pub fn bridge_gone(err: &str) -> bool {
    let status_hint = err.starts_with("HTTP 404") || err.starts_with("HTTP 405") || err.starts_with("HTTP 410");
    // 站点业务错误也会借 410 返回，带信封（含 "status"）的说明桥还在，不兜底
    status_hint && !err.contains("\"status\"")
}

async fn post_json(path: &str, body: &Value) -> Result<Value, String> {
    let url = format!("{}{path}", crate::net::BASE);
    let resp = crate::net::client()
        .post(&url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Referer", format!("{}/", crate::net::BASE))
        .json(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", &text[..text.len().min(160)]));
    }
    serde_json::from_str(&text).map_err(|e| format!("{e}: {}", &text[..text.len().min(160)]))
}

async fn open(scope: &'static str) -> Result<Session, String> {
    let secret = EphemeralSecret::random(&mut rand::rngs::OsRng);
    let client_pub = secret.public_key().to_encoded_point(false).as_bytes().to_vec();
    let hello = post_json(
        &format!("/api/{scope}/request"),
        &json!({ "v": 1, "type": "hello", "client_pub": b64u(&client_pub) }),
    )
    .await?;

    let kid = hello.get("kid").and_then(|v| v.as_str()).ok_or("加密握手响应无效")?.to_string();
    let sid = hello.get("sid").and_then(|v| v.as_str()).ok_or("加密握手响应无效")?.to_string();
    let site = hello.get("site").and_then(|v| v.as_str()).ok_or("加密握手响应无效")?.to_string();
    let server_time = hello.get("server_time").and_then(|v| v.as_i64()).ok_or("加密握手响应无效")?;
    let expires_at = hello.get("expires_at").and_then(|v| v.as_i64()).ok_or("加密握手响应无效")?;
    let server_pub = unb64_url(hello.get("server_pub").and_then(|v| v.as_str()).ok_or("加密握手响应无效")?)?;
    let salt = unb64_url(hello.get("salt").and_then(|v| v.as_str()).ok_or("加密握手响应无效")?)?;
    if salt.len() != 32 {
        return Err("加密握手响应无效".to_string());
    }

    let peer = PublicKey::from_sec1_bytes(&server_pub).map_err(|_| "加密握手响应无效".to_string())?;
    let shared = secret.diffie_hellman(&peer);
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared.raw_secret_bytes().as_slice());
    let info = format!("{PROTOCOL}\n{scope}\n{site}\n{kid}\n{sid}");
    let mut req_key = [0u8; 32];
    let mut res_key = [0u8; 32];
    hk.expand(format!("{info}\nreq").as_bytes(), &mut req_key).map_err(|_| "派生密钥失败")?;
    hk.expand(format!("{info}\nres").as_bytes(), &mut res_key).map_err(|_| "派生密钥失败")?;

    let local = now();
    Ok(Session {
        info,
        kid,
        sid,
        seq: 0,
        offset: server_time - local,
        expires: local + (expires_at - server_time).max(0),
        req: Aes256Gcm::new_from_slice(&req_key).map_err(|_| "派生密钥失败")?,
        res: Aes256Gcm::new_from_slice(&res_key).map_err(|_| "派生密钥失败")?,
    })
}

async fn session(scope: &'static str) -> Result<Session, String> {
    if let Some(s) = SESSIONS.lock().unwrap().get(scope).cloned() {
        if s.expires > now() + 5 && s.seq < MAX_SEQ {
            return Ok(s);
        }
    }
    let s = open(scope).await?;
    SESSIONS.lock().unwrap().insert(scope, s.clone());
    Ok(s)
}

fn nonce(seq: u32) -> [u8; 12] {
    let mut iv = [0u8; 12];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}

/// 走加密通道调用一次接口，返回接口原始信封（{data,status,msg}）。
pub async fn call(method: &str, path: &str, body: Option<&Value>, token: Option<&str>) -> Result<Value, String> {
    let scope = scope_for(method, path).await;
    let s = match session(scope).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[edge] handshake failed: {e}");
            return Err(e);
        }
    };
    let seq = s.seq + 1;
    let ts = now() + s.offset;
    let iv = nonce(seq);
    let aad = |dir: &str| format!("{}\n{dir}\n{seq}\n{ts}", s.info).into_bytes();

    let mut request = json!({
        "method": method,
        "path": path,
        "body": body.map(|b| B64.encode(serde_json::to_vec(b).unwrap_or_default())).unwrap_or_default(),
        "content_type": if body.is_some() { "application/json" } else { "" },
    });
    if let Some(b) = body {
        if let Some(obj) = request.as_object_mut() {
            obj.insert("content_type".into(), json!("application/json"));
        }
        let _ = b;
    }
    let mut packet = json!({ "request": request });
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        packet["authorization"] = json!(format!("Bearer {t}"));
    }

    let plain = serde_json::to_vec(&packet).map_err(|e| e.to_string())?;
    let cipher = s
        .req
        .encrypt(Nonce::from_slice(&iv), Payload { msg: &plain, aad: &aad("req") })
        .map_err(|_| "加密请求失败".to_string())?;

    let outer = post_json(
        &format!("/api/{scope}/request"),
        &json!({ "v": 1, "type": "call", "kid": s.kid, "sid": s.sid, "seq": seq, "ts": ts, "ciphertext": b64u(&cipher) }),
    )
    .await
    .map_err(|e| {
        eprintln!("[edge] outer call failed: {e}");
        e
    })?;

    if outer.get("type").and_then(|v| v.as_str()) != Some("result") {
        let code = outer.get("error").and_then(|v| v.as_str()).unwrap_or("加密接口请求失败");
        if code == "session_expired" {
            SESSIONS.lock().unwrap().remove(scope);
        }
        return Err(code.to_string());
    }
    if outer.get("seq").and_then(|v| v.as_u64()) != Some(seq as u64) {
        return Err("加密接口响应不匹配".to_string());
    }

    let raw = unb64_url(outer.get("ciphertext").and_then(|v| v.as_str()).ok_or("加密接口响应不匹配")?)?;
    let plain = s
        .res
        .decrypt(Nonce::from_slice(&iv), Payload { msg: &raw, aad: &aad("res") })
        .map_err(|_| "解密响应失败".to_string())?;
    let inner: Value = serde_json::from_slice(&plain).map_err(|_| "加密接口响应格式无效".to_string())?;

    if let Some(st) = SESSIONS.lock().unwrap().get_mut(scope) {
        st.seq = seq;
    }

    let status = inner.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
    let text = inner
        .get("body")
        .and_then(|v| v.as_str())
        .map(|b| String::from_utf8_lossy(&unb64_std(b).unwrap_or_default()).to_string())
        .unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}: {}", &text[..text.len().min(160)]));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("{e}: {}", &text[..text.len().min(160)]))
}

/// 从站点首页的 __cloudfrontCompat 里取加密路由表（取一次缓存）。
async fn routes() -> Option<Value> {
    if let Some(v) = ROUTES.lock().unwrap().as_ref() {
        return Some(v.clone());
    }
    let url = format!("{}/", crate::net::BASE);
    let html = crate::net::client().get(&url).send().await.ok()?.text().await.ok()?;
    let at = html.find("\"encryptedAPIRoutes\"")?;
    let start = html[..at].rfind('{')?;
    let bytes = html.as_bytes();
    let mut depth = 0i32;
    let mut end = start;
    for (i, ch) in bytes.iter().enumerate().skip(start) {
        match ch {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let snippet = &html[start..end];
    let cfg: Value = serde_json::from_str(snippet).ok()?;
    let table = cfg.get("encryptedAPIRoutes")?.clone();
    *ROUTES.lock().unwrap() = Some(table.clone());
    Some(table)
}

fn matches(pattern: &str, method: &str, segs: &[&str]) -> bool {
    let mut it = pattern.splitn(2, ' ');
    let pm = it.next().unwrap_or("");
    let pp = it.next().unwrap_or("");
    if pm != method {
        return false;
    }
    let ps: Vec<&str> = pp.split('/').collect();
    if ps.len() != segs.len() {
        return false;
    }
    ps.iter().zip(segs).all(|(p, s)| p.starts_with(':') || p == s)
}

async fn scope_for(method: &str, path: &str) -> &'static str {
    let m = if method.eq_ignore_ascii_case("HEAD") { "GET" } else { method };
    let no_query = path.split('?').next().unwrap_or(path);
    let segs: Vec<&str> = no_query.split('/').collect();
    if let Some(table) = routes().await {
        for scope in ["user", "playback", "dynamic", "public"] {
            if let Some(list) = table.get(scope).and_then(|v| v.as_array()) {
                for entry in list.iter().filter_map(|v| v.as_str()) {
                    if matches(entry, m, &segs) {
                        if scope == "public" && path.contains("sort=random") {
                            return "dynamic";
                        }
                        return match scope {
                            "user" => "user",
                            "playback" => "playback",
                            "dynamic" => "dynamic",
                            _ => "public",
                        };
                    }
                }
            }
        }
    }
    "public"
}
