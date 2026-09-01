// 前端遥测入口：只负责"发生了什么"，采什么、发不发由 Rust 侧决定。
//
// 这里刻意不做任何本地缓存或开关判断——开关在 mole-telemetry 里，前端多存
// 一份状态只会带来"UI 显示已关闭但其实还在发"的不一致。

import { invoke } from "@tauri-apps/api/core";
import type { TrackRequest } from "./types";

/** 上报一个事件。永不抛错：遥测挂了不该影响任何用户操作。 */
export function track(event: TrackRequest): void {
  void invoke("telemetry_track", { event }).catch(() => {});
}

/** 当前路由对应的界面名，用于 view_opened 与 error_occurred。 */
export function currentView(): string {
  const hash = location.hash.replace(/^#\//, "");
  return hash.split("?")[0] || "clean";
}

/** 计时器：返回一个调用即得毫秒数的函数，用于 scan_completed。 */
export function stopwatch(): () => number {
  const started = performance.now();
  return () => Math.round(performance.now() - started);
}
