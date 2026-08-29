---
title: "~/Library/Caches 能不能整个删掉"
description: "可以，但不该。这篇说明这个目录里到底有什么、哪几类删了会有真实代价、正在运行的 App 为什么是例外，以及怎么安全地清。"
lang: zh
permalink: can-you-delete-library-caches
translationKey: can-you-delete-library-caches
category: space
tags: ["磁盘空间", "缓存"]
publishedAt: 2026-08-10
updatedAt: null
readingMinutes: null
author: null
cover: null
coverAlt: null
featured: true
draft: false
noindex: false
canonical: null
toc: true
cta: source
series: null
seriesOrder: null
related: []
---

## 简短回答

技术上可以：`~/Library/Caches` 里的东西按定义都是可重建的，删光之后 macOS 和绝大多数 App 都还能正常启动。

但整个删是个笨办法。它会让你付出一些本来不必付的代价——重新登录、重新下载、下一次构建慢十几分钟——而这些代价换来的空间，其实集中在少数几个目录里。

## 这个目录里到底是什么

`~/Library/Caches` 是用户级缓存的约定位置，每个 App 用自己的 bundle identifier 建一个子目录。你可以先看一眼分布：

```bash
du -sh ~/Library/Caches/* 2>/dev/null | sort -rh | head -20
```

典型情况下前几名会是这几类：

| 目录 | 里面是什么 | 删掉的代价 |
| --- | --- | --- |
| `com.apple.dt.Xcode`、`org.swift.swiftpm` | Xcode 与 SwiftPM 的构建产物、下载的依赖 | 下一次构建明显变慢，需要重新拉依赖 |
| `Homebrew` | 下载过的 bottle 压缩包 | 重装同一个包时需要重新下载 |
| `com.google.Chrome`、`Firefox` 等 | 浏览器磁盘缓存 | 前几次访问慢一点；登录状态与历史记录不在这里 |
| `Figma`、`Sketch` 等 | 设计工具的资源与字体缓存 | 重新拉取一次；作品文件不在这里 |
| `com.tencent.xinWeChat` 等 | 聊天软件的图片与视频缓存 | 旧图片视频需要重新下载 |

## 哪几类删了基本没代价

- **浏览器缓存**：Cookie、登录态、历史记录保存在 `~/Library/Application Support` 下，不在 Caches 里。删缓存不会把你登出。
- **日志类缓存**：诊断用途，App 会按需重建。
- **残留的安装包**：这个严格说不在 Caches 里，但常常和缓存一起被忽略——下载文件夹和各处残留的 `.dmg` / `.pkg` / `.xip` 往往比缓存还大，而且删了完全没代价。

## 哪几类删了有真实代价

- **Xcode 相关**：`DerivedData` 和 SwiftPM 的缓存删掉之后，下一次 clean build 可能要多花十几分钟。空间紧张时值得删，日常不值得。
- **Homebrew 的 bottle 缓存**：`brew cleanup` 本来就会处理，手动删没问题，只是重装时要重新下载。
- **正在运行的 App 的缓存**：见下一节。

## 正在运行的 App 是个例外

这是整个目录里唯一真正会出问题的地方。一个 App 正在运行时，它可能持有缓存文件的句柄，也可能在内存里维护着和缓存内容对应的索引。这时候把文件从底下抽走，轻则下次启动时缓存重建，重则 App 当场出现找不到资源的错误状态。

所以正确顺序是：**先退出 App，再清它的缓存**。

在 Finder 里这一步很难做对，因为你看不出哪个目录属于哪个正在运行的进程。命令行可以查：

```bash
lsof +D ~/Library/Caches/com.example.app 2>/dev/null | head
```

但对着二十几个目录一个个查显然不现实。

## Tidy 在这里省掉的那一步

[Tidy](/zh/) 的清理页把上面这套判断做成了扫描结果的一部分：候选按 App 缓存、日志、开发工具、AI 工具、浏览器、设计工具、通讯工具、安装包分组列出，每一项标明体积；**正在运行的应用会被单独标出来**，并写清楚"关闭 微信 后可再清理 2.2 GB"这样的具体数字，而不是笼统地提示你注意。

关键在于执行边界：扫描结果会被存下来，执行阶段只接受你在预览里勾选的那个子集，确认之后不会重新扫描。也就是说，你看到的那份清单就是最终会被删掉的东西，不多不少。删除走废纸篓，删错了能拿回来。

> [!NOTE]
> 系统级缓存（`/Library/Caches` 而不是 `~/Library/Caches`）需要特权助手。这部分传输层还没发布，所以 Tidy 目前会直接以 `requires_admin` 拒绝，而不是弹一个 sudo 提示框。

## 一个务实的做法

1. 先看分布，确认空间到底被谁吃掉了；
2. 优先清浏览器、通讯工具、设计工具这类删了没代价的；
3. 清理残留的安装包，这一项通常性价比最高；
4. 空间实在紧张时再动 Xcode 和 Homebrew，并且知道下次构建会慢；
5. 动任何 App 的缓存之前先把它退出。

另一个经常被忽略的空间来源是卸载残留：[把 App 拖进废纸篓之后，硬盘上还剩下什么](/zh/blog/what-remains-after-uninstall)。
