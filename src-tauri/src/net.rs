//! 网络层：与上游站点对话。
//!
//! 站点把全部 `/api/**` 调用收敛成 `POST /api/edge/request` 的信封，
//! 我们只实现这一个入口，其余接口都是「填 path」。

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

pub const BASE: &str = "https://huangguoai.com";

pub const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// 应用内代理：空 = 直连（仍沿用系统/环境变量代理）。
static PROXY: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));
static PROXY_FILE: Lazy<RwLock<Option<PathBuf>>> = Lazy::new(|| RwLock::new(None));

static CLIENT: Lazy<RwLock<reqwest::Client>> = Lazy::new(|| RwLock::new(build_client(None)));

fn build_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .gzip(true)
        .brotli(true)
        .pool_idle_timeout(Duration::from_secs(60));
    if let Some(p) = proxy {
        if let Ok(px) = reqwest::Proxy::all(p) {
            builder = builder.proxy(px);
        }
    }
    builder
        .build()
        .expect("build http client")
}

pub(crate) fn client() -> reqwest::Client {
    CLIENT.read().unwrap().clone()
}

/// 当前代理设置，空串表示直连。
pub fn proxy() -> String {
    PROXY.read().unwrap().clone().unwrap_or_default()
}

/// 启动时读取已保存的代理。
pub fn bind(dir: &Path) {
    let path = dir.join("proxy.json");
    let saved = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("proxy").and_then(|p| p.as_str()).map(str::to_string))
        .filter(|p| !p.trim().is_empty());
    if let Some(p) = saved.as_deref() {
        *CLIENT.write().unwrap() = build_client(Some(p));
    }
    *PROXY.write().unwrap() = saved;
    *PROXY_FILE.write().unwrap() = Some(path);
}

/// 设置代理并立即生效；空串表示直连。设置失败不会影响当前生效的客户端。
pub fn set_proxy(input: &str) -> Result<(), String> {
    const BAD: &str = "代理地址无效，示例：http://127.0.0.1:7890";
    let value = input.trim();
    let normalized = if value.is_empty() {
        None
    } else {
        let parsed = url::Url::parse(value).map_err(|_| BAD.to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("仅支持 http:// 或 https:// 代理".to_string());
        }
        if parsed.host_str().unwrap_or("").is_empty() {
            return Err("代理地址缺少主机名".to_string());
        }
        reqwest::Proxy::all(value).map_err(|_| BAD.to_string())?;
        Some(value.to_string())
    };
    let next = build_client(normalized.as_deref());
    *CLIENT.write().unwrap() = next;
    *PROXY.write().unwrap() = normalized.clone();
    if let Some(path) = PROXY_FILE.read().unwrap().as_ref() {
        let _ = std::fs::write(path, json!({ "proxy": normalized.unwrap_or_default() }).to_string());
    }
    Ok(())
}

/// 站点偶发 TLS 抖动（实测），统一重试。
async fn with_retry<F, Fut, T>(mut f: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut last = String::from("network error");
    for attempt in 0..3 {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = e;
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(350 * (attempt as u64 + 1))).await;
                }
            }
        }
    }
    Err(last)
}

/// 业务请求：走边缘信封。
pub async fn edge(
    method: &str,
    path: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> Result<Value, String> {
    if crate::edge::forced() {
        let raw = crate::edge::call(method, path, body.as_ref(), token).await?;
        return unwrap_envelope(raw);
    }
    match legacy(method, path, body.clone(), token).await {
        Ok(v) => Ok(v),
        Err(e) if crate::edge::bridge_gone(&e) => {
            eprintln!("[net] legacy bridge gone ({e}), retry via encrypted channel");
            let raw = crate::edge::call(method, path, body.as_ref(), token).await?;
            match unwrap_envelope(raw.clone()) {
                Ok(v) => Ok(v),
                Err(e) => {
                    eprintln!("[net] encrypted {method} {path} -> {e} | raw={}", &raw.to_string()[..raw.to_string().len().min(200)]);
                    Err(e)
                }
            }
        }
        Err(e) => {
            eprintln!("[net] {method} {path} -> {e}");
            Err(e)
        }
    }
}

/// 收紧响应信封：status=1 取 data，否则拿 msg 当错误。
fn unwrap_envelope(value: Value) -> Result<Value, String> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    if value.get("status").and_then(|v| v.as_i64()) == Some(1) {
        Ok(value.get("data").cloned().unwrap_or(Value::Null))
    } else {
        Err(value
            .get("msg")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("接口返回失败")
            .to_string())
    }
}

/// 旧明文桥 POST /api/edge/request（站点仍启用）。
async fn legacy(
    method: &str,
    path: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> Result<Value, String> {
    let (encoded, ctype) = match &body {
        Some(v) => (
            B64.encode(serde_json::to_vec(v).map_err(|e| e.to_string())?),
            "application/json",
        ),
        None => (String::new(), ""),
    };
    let envelope = json!({
        "method": method,
        "path": path,
        "body": encoded,
        "content_type": ctype,
    });

    let url = format!("{BASE}/api/edge/request");
    let token = token.map(|s| s.to_string());

    let value = with_retry(|| {
        let envelope = envelope.clone();
        let token = token.clone();
        let url = url.clone();
        async move {
            let mut req = client()
                .post(&url)
                .header("Accept", "application/json")
                .header("Referer", format!("{BASE}/"))
                .json(&envelope);
            if let Some(t) = token.as_deref() {
                if !t.is_empty() {
                    req = req.header("Authorization", format!("Bearer {t}"));
                }
            }
            let resp = req.send().await.map_err(|e| e.to_string())?;
            let status = resp.status();
            let text = resp.text().await.map_err(|e| e.to_string())?;
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err("401".to_string());
            }
            if !status.is_success() {
                return Err(format!("HTTP {status}: {}", &text[..text.len().min(160)]));
            }
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_str::<Value>(&text).map_err(|e| format!("{e}: {}", &text[..text.len().min(160)]))
        }
    })
    .await?;

    unwrap_envelope(value)
}

/// 拉原始字节（图片 / m3u8 / 分片）。
pub async fn fetch_bytes(url: &str) -> Result<(Option<String>, Vec<u8>), String> {
    let url = url.to_string();
    with_retry(|| {
        let url = url.clone();
        async move {
            let resp = client()
                .get(&url)
                .header("Referer", format!("{BASE}/"))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status()));
            }
            let ctype = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let bytes = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
            Ok((ctype, bytes))
        }
    })
    .await
}
