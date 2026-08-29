---
title: "把 App 拖进废纸篓之后，硬盘上还剩下什么"
description: "macOS 的拖拽卸载只删掉 .app 本体。这篇列出十类常见残留、各自的位置，以及为什么有些应用必须用官方卸载器。"
lang: zh
permalink: what-remains-after-uninstall
translationKey: what-remains-after-uninstall
category: apps
tags: ["磁盘空间", "卸载"]
publishedAt: 2026-08-14
updatedAt: null
readingMinutes: null
author: null
cover: null
coverAlt: null
featured: false
draft: false
noindex: false
canonical: null
toc: true
cta: source
series: null
seriesOrder: null
related: []
---

## 拖拽卸载只做了一件事

把 `/Applications/Example.app` 拖进废纸篓，删掉的就是这个 bundle 本身。App 在安装和使用过程中写到别处的东西，一样不少地留着。

对一个用了两年的应用，残留常常比 bundle 本身还大。

## 十类常见残留

| 类别 | 典型位置 |
| --- | --- |
| App Support | `~/Library/Application Support/<bundle-id>` |
| 缓存 | `~/Library/Caches/<bundle-id>` |
| 日志 | `~/Library/Logs/<app>` |
| 偏好设置 | `~/Library/Preferences/<bundle-id>.plist` |
| Launch Agents | `~/Library/LaunchAgents/<bundle-id>.plist` |
| HTTP 存储 | `~/Library/HTTPStorages/<bundle-id>` |
| WebKit | `~/Library/WebKit/<bundle-id>` |
| Saved State | `~/Library/Saved Application State/<bundle-id>.savedState` |
| 容器 | `~/Library/Containers/<bundle-id>`、`~/Library/Group Containers/<group-id>` |
| 其它 | 应用自定义的目录，命名不一定跟 bundle id 相关 |

最后一类是手动清理最容易漏的：不是所有应用都规规矩矩用 bundle identifier 命名，有些会在 `~/Documents` 或 `~/Library` 下建一个人类可读的目录名。

## 为什么不能只按名字搜

一个很常见的做法是拿应用显示名去全盘 `find`，然后把命中的都删掉。这个做法有两个问题：

第一，**显示名会误伤**。搜 "Notes" 会命中一堆和 Apple 备忘录无关的目录。

第二，**同一个厂商的应用共用目录**。删掉 `~/Library/Group Containers/group.com.example` 可能同时影响你还在用的另一个应用。

可靠的做法是从已安装应用的 `Info.plist` 里读出真实的 `CFBundleIdentifier`，再按这个标识去匹配，而不是按屏幕上显示的名字。

## 有些应用必须用官方卸载器

以下几类不要手动删：

- **装了系统扩展或内核扩展的**：VPN 客户端、虚拟机、杀毒软件、部分驱动。手动删掉主体会留下已注册的扩展，可能导致开机异常。
- **装了特权守护进程的**：`/Library/LaunchDaemons` 与 `/Library/PrivilegedHelperTools` 下的东西属于系统范围，需要管理员权限，也需要按正确顺序注销。
- **Mac App Store 安装的沙盒应用**：从 Launchpad 删除会一并处理容器，比手动删干净。

## Tidy 在这里的做法

[Tidy](/zh/) 的「软件」页读取已安装应用清单，按上面这十类把每个残留文件列出来，标明各自体积；正在运行的应用、系统组件、以及"必须使用官方卸载器"的应用会被挡下来并写明原因，而不是让你先删了再说。

它不会用显示名做通配匹配——保护名单在编译期由 `build.rs` 从数据文件生成成 Rust 常量，遇到无法识别的行直接让构建失败，所以名单本身不会因为一次误编辑而悄悄放宽。名单的 `DATA_SHA256` 显示在关于页，可以核对。

> [!WARNING]
> 系统范围的卸载（`/Library` 下的守护进程与特权助手工具）目前不会被执行。特权助手的传输层还没发布，这类操作会以 `requires_admin` 直接拒绝，不会退回到 shell 提权。这类残留仍然需要你用官方卸载器处理。

删除同样走废纸篓，执行范围严格等于你在预览里勾选的那一份，逐项都能展开复核。

相关：[~/Library/Caches 能不能整个删掉](/zh/blog/can-you-delete-library-caches)。
