# 流光 1.0.2：依赖精简与验证

日期：2026-11-15。平台：Windows x64。本版精简媒体工具的重复依赖，保留原界面、配色、下载功能和私有部署方式。压缩算法仍为上一版的 solid zlib。

## 实际减少的文件

1.0.1 的 FFmpeg、FFprobe 各自带有完整媒体库。1.0.2 改用同一上游发布、同一源码修订的 GPL shared 发行版：两个程序共用应用 `engine/` 内的 7 个 DLL。

| 媒体工具文件 | 体积 |
| --- | ---: |
| 原两个静态 EXE | 330,678,272 字节（315.36 MiB） |
| 新两个 EXE、7 个私有 DLL | 199,772,160 字节（190.52 MiB） |
| 实际减少 | **130,906,112 字节（124.84 MiB，39.59%）** |

DLL 通过程序自身目录加载，不写入系统目录、不注册共享组件、不修改用户或系统 PATH。完整 WebView2、Node、yt-dlp、命令行入口及高级参数继续保留。整个应用和安装包还包含其他运行时与界面，最终大小以交付清单为准。

## 等价性验证

候选来自[同一上游发布](https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/tag/autobuild-2026-11-14-12-37)，文件名为 `ffmpeg-n8.1.3-35-gf1acb0a266-win64-gpl-shared-8.1.zip`，89,659,320 字节，SHA-256 为 `94dd295461cada31a9e7f79ec0f9ec317d9afcbfe9e60b337679bce2c5a60016`。

- 归档 SHA-256 和 ZIP CRC 通过。
- 9 个 EXE/DLL 的常规与延迟导入均完成核对，必需依赖由随包 DLL 和 Windows 系统库构成。
- 两个程序的版本、编译器、媒体库 ABI 一致；配置仅增加 `--enable-shared --disable-static`。
- 16 类能力列表逐字一致：编码器、解码器、编解码器、格式、解复用器、复用器、滤镜、协议、位流滤镜、硬件加速类型、设备、像素格式、采样格式、声道布局、处置标记、颜色。
- 两个程序的完整命令行帮助逐字一致。
- 合成 H.264/AAC 视频编码、FFprobe 读取、转封装和解码通过，新旧解码帧校验一致。
- 子进程临时文件限定在项目测试目录，PATH 仅含 Windows System32，避免缺失 DLL 被开发机的其他安装意外补齐。

复核入口为 `scripts/verify-ffmpeg-shared.py`。不访问用户视频、Cookies 或代理配置，不运行安装器。硬件加速仅比较编译能力枚举，没有执行真实 GPU 编码；PE 检查不代表穷尽可选硬件驱动的动态加载。

## 保留其他依赖的原因

当前界面使用 Tauri/React。微软要求分发完整 Fixed Runtime，没有官方支持的组件裁剪清单，因此保留完整私有 WebView2。复用已有系统 Runtime 会改变缺失运行时机器的首次启动和离线分发方式；彻底移除浏览器运行时需要重写界面层。[微软分发要求](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#the-fixed-version-runtime-distribution-mode)

QuickJS-NG 虽然更小，但其现成 yt-dlp provider 不具备当前 Node 参数提供的同等权限限制，也不是任意 Node 脚本和显式 Node 参数的等价替代。本轮保留 Node。Node 权限机制本身也不是完整恶意代码沙箱。[Node 权限说明](https://github.com/nodejs/node/blob/v24.15.0/doc/api/permissions.md)

## 打包与验收边界

release 主程序、CLI 与 NSIS 安装包构建成功。最终文件为 `liuguang-1.0.2-windows-x64-setup.exe`，454,836,139 字节（433.77 MiB），SHA-256 为 `29f9efdf79d4971c113c76f3725f33763055070368d59223f44c33c2ebf86c51`，未签名。

| 对比 | 1.0.1 | 1.0.2 | 减少 |
| --- | ---: | ---: | ---: |
| 安装包 | 480.59 MiB | 433.77 MiB | 46.83 MiB（9.74%） |
| 实际入包文件（未压缩） | 1,145.40 MiB | 1,020.56 MiB | 124.83 MiB |

两版均使用 solid zlib。安装文件的减少比媒体工具本身节省量略小，因为新增 DLL 需要清单与校验元数据。未压缩体积按实际 NSIS 的唯一目标路径统计，不包含运行后缓存和文件系统分配开销。

本版已通过：

- 安装器契约 10 项、生命周期与窗口边界 13 项，以及桌面 all-targets 严格 Clippy。
- 应用真实本地媒体回归：中文空格目录、下载、独立音视频合并、严格画质与显式降级、音频提取、中英字幕及筛选、MP4 字幕嵌入、现有文件保护、预设、原生 JSON 输出、合集解析与增量事件、重启后的设置与历史恢复。
- 重启验证改为比较实际目录，兼容 Windows 路径分隔符规范化；业务代码未因此修改。
- 实际 NSIS 中 2,470 项锁定运行资源逐项核对路径和 SHA-256，7 个媒体 DLL 均进入安装与卸载清单；CLI helper 仅依赖 Windows 系统 DLL，预检不需要加载媒体库或 WebView2。
- 与上一版源码归档比较，35 个界面及 Rust 业务源码文件一致；网页版 7 个基线文件哈希不变。

冻结资源清单 SHA-256 为 `79d43662bf4190cbb55a3c0e120a3aa1442d4520bc723fbb8666d912802cdce8`。源码材料包的 3,096 个内容文件均经 CRC/SHA 校验，其 SHA-256 为 `79ace78dcf446ae6fdcd201a703c77bcb628a421776b12f68051833aa26a5308`。完整交付文件和校验和由 `release/release-manifest.json`、`release/SHA256SUMS.txt` 记录。

1.0.1 的历史安全回归见 [上一版验证](verification-1.0.1.md)，未将历史结果冒充本版重测。

安装、升级和卸载实测继续按用户要求不执行。本轮不在系统中试装，不启动桌面 GUI，不修改网页版。
