import { useEffect, useRef, useState } from "react";
import { api, fmt } from "../api";
import type { Post, VideoCard } from "../types";
import { useStore } from "../store";
import { Masonry, RowCard } from "../components/cards";
import { Empty, Img } from "../components/ui";
import { IBack, IChevR, IGrid, ITrophy } from "../icons";

export const CHANNELS = [
  { cate: "ai-duanju", name: "AI成人短剧", desc: "高能反转 · 一口气看完" },
  { cate: "ai-manju", name: "AI成人漫剧", desc: "国创漫剧 · 每周更新" },
  { cate: "ai-huanlian", name: "AI换脸", desc: "换脸演绎 · 全新剧情" },
  { cate: "ai-mogai", name: "AI魔改", desc: "魔改名场面 · 脑洞大开" },
] as const;

/* 远端只给名称与路由，副标题按路由取本地文案 */
const DESC: Record<string, string> = Object.fromEntries(CHANNELS.map((c) => [c.cate, c.desc]));
type Channel = { cate: string; name: string; desc: string };

/* ================= 分类总览 ================= */
export default function Category({ wide }: { wide: boolean }) {
  const { push } = useStore();
  const [channels, setChannels] = useState<Channel[]>(CHANNELS.map((c) => ({ ...c })));
  const [rows, setRows] = useState<Record<string, VideoCard[]>>({});
  const [posts, setPosts] = useState<Post[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    (async () => {
      /* 分类表以站点为准：上游下线/改名时这里自动跟随 */
      let list: Channel[] = CHANNELS.map((c) => ({ ...c }));
      try {
        const remote = (await api.siteCategories())?.items || [];
        if (remote.length) {
          list = remote.map((i) => ({ cate: i.route, name: i.name, desc: DESC[i.route] ?? "" }));
        }
      } catch { /* 保留内置频道 */ }
      const entries = await Promise.all(
        list.map(async (c) => {
          try {
            const res = await api.category(c.cate, 1, 8);
            return [c.cate, res?.items || []] as const;
          } catch {
            return [c.cate, [] as VideoCard[]] as const;
          }
        }),
      );
      let p: Post[] = [];
      try { p = (await api.posts(1, 8))?.items || []; } catch { p = []; }
      if (!alive) return;
      setChannels(list);
      setRows(Object.fromEntries(entries));
      setPosts(p);
      setLoading(false);
    })();
    return () => { alive = false; };
  }, []);

  return (
    <div className="absolute inset-0 bg-bg flex flex-col">
      <div className="shrink-0 px-4 pb-1" style={{ paddingTop: "calc(var(--sat, 0px) + 14px)" }}>
        <div className="flex items-end gap-2.5">
          <span className="text-[24px] font-bold text-on leading-none">分类</span>
          <span className="text-[11px] text-mut pb-0.5">{channels.length + 1} 个频道 · 每周持续上新</span>
        </div>
      </div>

      <div className="flex-1 scroll-y pb-24 pt-2">
        {/* 排行榜入口 */}
        <button onClick={() => push({ kind: "ranks" })}
          className="mx-4 mb-5 w-[calc(100%-2rem)] flex items-center gap-3 rounded-(--r-md) bg-s2 p-3.5 text-left pressable elev-1">
          <span className="w-10 h-10 rounded-(--r-sm) bg-pc text-on-pc flex items-center justify-center shrink-0"><ITrophy size={20} /></span>
          <span className="flex-1">
            <span className="block text-[14.5px] font-semibold text-on">排行榜</span>
            <span className="block text-[10.5px] text-mut mt-0.5">热播榜 · 推荐榜 · 潜力榜</span>
          </span>
          <IChevR size={16} className="text-mut" />
        </button>

        {channels.map((c, ci) => {
          const list = rows[c.cate] || [];
          return (
            <section key={c.cate} className="mb-5 a-rise" style={{ animationDelay: `${Math.min(ci * 50, 300)}ms` }}>
              <button onClick={() => push({ kind: "channel", cate: c.cate, name: c.name })}
                className="w-full flex items-center gap-2.5 px-4 mb-2.5 text-left pressable group">
                <span className="w-[36px] h-[36px] rounded-(--r-sm) tonal flex items-center justify-center shrink-0 text-primary">
                  <IGrid size={19} strokeWidth={1.9} />
                </span>
                <span className="flex-1 min-w-0">
                  <span className="block text-[15.5px] font-semibold text-on leading-tight">{c.name}</span>
                  <span className="block text-[10.5px] text-mut mt-0.5 truncate">{c.desc}</span>
                </span>
                <span className="flex items-center gap-0.5 text-[11.5px] text-mut group-hover:text-primary transition-colors">
                  全部<IChevR size={14} />
                </span>
              </button>
              <div className="scroll-x flex gap-2.5 px-4 snap-x">
                {list.map((v) => (
                  <RowCard key={v.id} v={v} onOpen={() => push({ kind: "feed", videoIds: list.map((x) => x.id), index: list.findIndex((x) => x.id === v.id), title: c.name })} />
                ))}
                {!list.length && <div className="h-[150px] flex items-center text-[11.5px] text-dim px-1">{loading ? "加载中…" : "暂无内容"}</div>}
              </div>
            </section>
          );
        })}

        {/* 吃瓜 */}
        <section className="mb-5 a-rise">
          <button onClick={() => push({ kind: "posts" })}
            className="w-full flex items-center gap-2.5 px-4 mb-2.5 text-left pressable group">
            <span className="w-[36px] h-[36px] rounded-(--r-sm) tonal flex items-center justify-center shrink-0 text-primary">
              <IGrid size={19} strokeWidth={1.9} />
            </span>
            <span className="flex-1 min-w-0">
              <span className="block text-[15.5px] font-semibold text-on leading-tight">黄果吃瓜</span>
              <span className="block text-[10.5px] text-mut mt-0.5 truncate">圈内爆料 · 每日更新</span>
            </span>
            <span className="flex items-center gap-0.5 text-[11.5px] text-mut group-hover:text-primary transition-colors">
              全部<IChevR size={14} />
            </span>
          </button>
          <div className="scroll-x flex gap-2.5 px-4 snap-x">
            {posts.map((post) => (
              <button key={post.id} onClick={() => push({ kind: "post", id: post.id })}
                className="w-[104px] shrink-0 text-left snap-start pressable">
                <div className="relative w-full rounded-(--r-sm) overflow-hidden bg-s1 elev-1" style={{ paddingTop: "133%" }}>
                  <Img src={post.cover} />
                  <div className="absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-t from-black/75 to-transparent" />
                  <span className="absolute bottom-1.5 right-1.5 text-[9px] font-semibold text-white/90 tsh">{fmt(post.hot)}</span>
                </div>
                <div className="mt-1.5 text-[11.5px] font-medium text-on clamp-1 leading-tight">{post.title}</div>
              </button>
            ))}
            {!posts.length && <div className="h-[150px] flex items-center text-[11.5px] text-dim px-1">{loading ? "加载中…" : "暂无内容"}</div>}
          </div>
        </section>

        <div className="px-4 text-center text-[10.5px] text-dim pt-1">— 已展示全部频道 —</div>
      </div>
      {wide && <div />}
    </div>
  );
}

/* ================= 频道详情页 ================= */
export function ChannelPage({ cate, name, onBack, wide }: { cate: string; name: string; onBack: () => void; wide: boolean }) {
  const { push } = useStore();
  const [sorts, setSorts] = useState<{ name: string; route: string }[]>([]);
  const [sortRoute, setSortRoute] = useState("");
  const [items, setItems] = useState<VideoCard[]>([]);
  const [page, setPage] = useState(0);
  const [pages, setPages] = useState(1);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [offline, setOffline] = useState<string | null>(null);
  const busy = useRef(false);
  const sentinel = useRef<HTMLDivElement>(null);

  const loadMore = async () => {
    if (busy.current || page >= pages) return;
    busy.current = true;
    setLoading(true);
    try {
      const next = page + 1;
      const res = await api.category(cate, next, 20, sortRoute || undefined);
      setItems((prev) => {
        const base = next === 1 ? [] : prev;
        const seen = new Set(base.map((x) => x.id));
        return [...base, ...(res?.items || []).filter((x: VideoCard) => !seen.has(x.id))];
      });
      setPage(next);
      setPages(res?.pagination?.pages || next);
      setErr(null);
    } catch (e) {
      const msg = (e as Error).message || "加载失败";
      /* 上游清空分类时返回「内容已下线」，按空列表处理而不是报错 */
      if (/已下线|不存在/.test(msg)) {
        setOffline(msg);
        setItems([]);
        setPage(1);
        setPages(1);
        setErr(null);
      } else {
        setErr(msg);
      }
    } finally {
      busy.current = false;
      setLoading(false);
    }
  };

  useEffect(() => {
    api.siteSorts()
      .then((r) => setSorts((r?.items || []).filter((s) => s.route !== "watchings")))
      .catch(() => {});
  }, []);

  useEffect(() => {
    setItems([]); setPage(0); setPages(1); setErr(null); setOffline(null);
    loadMore();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sortRoute]);
  useEffect(() => {
    const el = sentinel.current;
    if (!el) return;
    const io = new IntersectionObserver((es) => { if (es[0].isIntersecting) loadMore(); }, { rootMargin: "600px" });
    io.observe(el);
    return () => io.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, pages]);

  return (
    <div className="absolute inset-0 bg-bg flex flex-col">
      <div className="shrink-0 bg-s1" style={{ paddingTop: "var(--sat, 0px)" }}>
        <div className="px-3 pt-2 pb-3 flex items-center gap-2.5">
          <button onClick={onBack} className="w-10 h-10 rounded-full flex items-center justify-center text-on hover:bg-s3 pressable"><IBack size={20} /></button>
          <span className="w-[40px] h-[40px] rounded-(--r-sm) bg-pc text-on-pc flex items-center justify-center"><IGrid size={21} strokeWidth={1.9} /></span>
          <div className="flex-1 min-w-0">
            <div className="text-[17px] font-bold text-on leading-tight">{name}</div>
            <div className="text-[10.5px] text-mut">共 {items.length} 部 · 已加载 {page}/{Math.max(pages, 1)} 页</div>
          </div>
        </div>
        <div className="px-4 pb-3 flex gap-2 overflow-x-auto">
          {[{ name: "默认", route: "" }, ...sorts].map((s) => (
            <button key={s.route || "default"} onClick={() => setSortRoute(s.route)}
              className={`px-4 h-8 shrink-0 rounded-(--r-ctl) text-[12px] font-semibold pressable ${sortRoute === s.route ? "tonal text-on-sec" : "bg-s3 text-mut"}`}
              style={sortRoute === s.route ? { background: "var(--p-sec)", color: "var(--p-primary)" } : undefined}>{s.name}</button>
          ))}
        </div>
      </div>
      <div className="flex-1 scroll-y px-4 pt-3 pb-8">
        {items.length ? (
          <Masonry videos={items} wide={wide} onOpen={(v) => push({ kind: "feed", videoIds: items.map((x) => x.id), index: items.findIndex((x) => x.id === v.id), title: name })} />
        ) : offline ? (
          <Empty icon={<IGrid size={26} />} title="该分类暂无内容" sub="上游已下线该分类，恢复后会自动出现" />
        ) : err ? (
          <Empty icon={<IGrid size={26} />} title="加载失败" sub={err} action="重试" onAction={loadMore} />
        ) : (
          <div className="flex justify-center py-10"><div className="w-8 h-8 spinner" /></div>
        )}
        <div ref={sentinel} className="h-10" />
        {loading && <div className="flex justify-center py-3"><div className="w-6 h-6 spinner" /></div>}
        {!loading && page >= pages && items.length > 0 && (
          <div className="text-center text-[10.5px] text-dim py-3">— 已经到底啦 —</div>
        )}
      </div>
    </div>
  );
}
