// Tidy 遥测反向代理：把桌面端的批量上报转发给 PostHog。
//
// 存在的两个理由：
//   1. 可达性 —— app.posthog.com 在部分地区不稳，直连会让那部分用户的数据
//      整段丢失，DAU 看起来凭空少一截；走自己的域名就没这个问题。
//   2. 隐私 —— 这里是把客户端 IP 掐掉的地方。应用侧已经带了
//      $geoip_disable，但那只是"请求服务端别查"；真正的保证是 PostHog
//      根本拿不到 IP。
//
// 部署：npx wrangler deploy（见同目录 README.md）

/** 允许转发的路径。其余一律 404，代理不当通用转发器用。 */
const ALLOWED_PATHS = new Set(["/batch", "/batch/", "/e", "/e/", "/i/v0/e/"]);

/** 会泄露来源地址的请求头，转发前全部去掉。 */
const IP_HEADERS = [
  "cf-connecting-ip",
  "x-forwarded-for",
  "x-real-ip",
  "true-client-ip",
  "cf-ipcountry",
  "cf-ipcity",
  "cf-iplatitude",
  "cf-iplongitude",
  "cf-ipcontinent",
];

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: corsHeaders() });
    }
    // 只收 POST：这个端点不提供任何可读取的东西，GET 一律拒绝。
    if (request.method !== "POST" || !ALLOWED_PATHS.has(url.pathname)) {
      return new Response("not found", { status: 404 });
    }

    const upstream = new URL(env.POSTHOG_HOST || "https://us.i.posthog.com");
    upstream.pathname = url.pathname === "/batch" ? "/batch/" : url.pathname;

    const headers = new Headers(request.headers);
    for (const name of IP_HEADERS) headers.delete(name);
    headers.delete("cookie");
    headers.set("host", upstream.host);

    try {
      const response = await fetch(upstream.toString(), {
        method: "POST",
        headers,
        body: request.body,
      });
      // 只把状态码传回去。上游返回体对客户端没有用，转发它只会让一个
      // 上游故障变成客户端要处理的新格式。
      return new Response(response.ok ? "ok" : "upstream error", {
        status: response.ok ? 200 : 502,
        headers: corsHeaders(),
      });
    } catch {
      return new Response("upstream unreachable", {
        status: 502,
        headers: corsHeaders(),
      });
    }
  },
};

/** 桌面端是 Rust 侧发的请求，用不到 CORS；这几行只为浏览器里调试方便。 */
function corsHeaders() {
  return {
    "access-control-allow-origin": "*",
    "access-control-allow-headers": "content-type",
    "access-control-allow-methods": "POST, OPTIONS",
  };
}
