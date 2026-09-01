# Tidy 遥测代理

一个 Cloudflare Worker，把桌面端的匿名统计转发给 PostHog，并在转发前抹掉客户端 IP。

## 为什么要这一跳

- **可达性**：`app.posthog.com` 在部分地区不稳。直连会让那部分用户的上报整段失败，日活看起来凭空少一截。
- **隐私**：应用发出的每条事件都带 `$geoip_disable`，但那只是请求服务端别查。真正的保证是 PostHog 拿不到 IP —— 这里就是掐掉 IP 的地方。

## 部署

```bash
cd tools/telemetry-proxy
npx wrangler deploy
```

然后在 Cloudflare 控制台把一个自定义域（例如 `t.example.com`）绑到这个 Worker，或者取消 `wrangler.toml` 里 `[[routes]]` 的注释。

## 接到桌面端

遥测地址和 PostHog 项目公钥都是**构建期**注入的。两个变量任意一个为空，遥测就整个编译不进二进制：

```bash
TIDY_TELEMETRY_URL=https://t.example.com \
TIDY_TELEMETRY_KEY=phc_xxxxxxxx \
make release
```

`TIDY_TELEMETRY_KEY` 是 PostHog 的 Project API Key（只写不读，天然公开，可以进构建产物）。

## 验证

```bash
curl -X POST https://t.example.com/batch/ \
  -H 'content-type: application/json' \
  -d '{"api_key":"phc_xxx","batch":[{"event":"app_launched","properties":{"distinct_id":"smoke-test"}}]}'
# 期望：ok
```

`GET` 任意路径应当返回 404 —— 这个端点不对外提供任何可读内容。

## 采了什么

字段清单以 `site/{zh,en}/privacy` 为准，代码在 `desktop/crates/mole-telemetry/src/event.rs`。改动采集面时，这三处必须一起改。
