# 流光 · Liuguang

基于 **Rust + Iced、yt-dlp 和 FFmpeg** 的原生视频下载桌面应用。

粘贴链接，选择视频或合集、画质和保存位置，即可管理下载。常用操作集中在主界面，完整参数目录、预设和原生命令工具放在高级功能中。界面使用暖灰、砖红和灰蓝配色，无需 WebView2 或浏览器后端。

## 功能

- 单视频、合集解析与选集，保留原始集号，支持范围选择和搜索。
- 分辨率与格式选择；指定画质默认不静默降级，文件大小区分准确值、估算值和未知。
- 下载队列、并发控制、取消、重试、历史记录及可继续的下载。
- 音视频合并、音频提取、字幕、章节、封面、元数据和片段处理。
- 代理、Cookies、下载参数预设、完整 yt-dlp 参数目录及独立 CLI。
- 下载视频与配置、缓存、临时文件分开保存；清理按应用数据归属保护视频。

## 平台与当前状态

本仓库是流光的**桌面版源码首发**，不包含网页版、运行二进制、安装包或个人数据。

| 平台 | 当前状态 |
| --- | --- |
| Windows x64 GNU | 已完成本地原生构建、选定回归和安装包编译；尚未完成完整界面及安装、升级、卸载实机验收 |
| macOS Apple Silicon | 已实现源码适配和独立打包流程；尚未完成 Mac 编译与运行验证 |
| macOS Intel | 已实现源码适配和独立打包流程；尚未完成 Mac 编译与运行验证 |

当前没有已验证的 Mac 安装包。Windows 的历史安装包测得 135.07 MiB；该数字不是未来构建的固定大小。验证范围见 [Windows 记录](docs/verification-2.0.0.md) 与 [macOS 说明](docs/macos.md)。

完整运行包会携带 yt-dlp、FFmpeg、FFprobe 和 Node，普通使用不需要额外安装 Rust、Python、Node、FFmpeg 或 WebView2。Node 用于下载引擎的 JavaScript 处理。视频下载仍需可访问目标网站的网络；认证、外部下载器及额外插件按实际需要配置。

## 从源码构建

在仓库根目录执行。需要已有 Rust、Python 3.11+ 和目标平台编译工具。脚本使用项目内缓存，不安装全局运行时、不修改系统 PATH，也不运行安装器。初次构建需要下载锁定的开发依赖和运行组件。

### Windows x64

已验证的工具链为 `x86_64-pc-windows-gnu`，需要 MinGW-w64；制作安装包还需要已有 NSIS 3。MSVC 尚未验证。

```powershell
$env:CARGO_HOME = Join-Path $PWD '.build/cargo'
$env:CARGO_TARGET_DIR = Join-Path $PWD 'target'
$env:TEMP = Join-Path $PWD '.build/tmp'
$env:TMP = $env:TEMP
New-Item -ItemType Directory -Force $env:CARGO_HOME,$env:TEMP | Out-Null
cargo fetch --locked --target x86_64-pc-windows-gnu
python -B scripts/prepare-native-engines.py
./scripts/build-native.ps1 -Offline -NoPackage
```

制作安装包时提供现有编译器的路径：

```powershell
./scripts/build-native.ps1 -Offline -Makensis 'C:\path\to\makensis.exe'
```

可通过 `-ToolDirectories` 指定现有工具所在目录。FFmpeg 两个程序及七个私有 DLL 必须保持同目录。详见 [Windows 打包说明](packaging/README.md)。

### macOS

需要对应架构的 Mac、Rust、Python 3.11+ 和 Xcode Command Line Tools。两个架构分别原生构建，不要求使用 Rosetta：

```sh
# Apple Silicon
python3 -B scripts/build-macos.py --target aarch64-apple-darwin
# Intel Mac
python3 -B scripts/build-macos.py --target x86_64-apple-darwin
```

流程生成包含私有运行依赖的 `.app` 和 DMG，默认使用 ad hoc 签名。Developer ID 与公证为可选步骤。当前最低构建版本设为 macOS 13.5，仍需实际构建检查确认，不能视为该系统版本已验证。详见 [Mac 打包说明](packaging/macos/README.md)。

三个 GitHub Actions 工作流均**仅手动触发**。Windows、Mac 构建默认不上传构建附件，不创建 Release，也不执行安装、升级或卸载。仓库创建和上传不代表这些工作流已经通过。

## 开发与验证

轻量打包边界检查无需下载运行组件：

```powershell
python -B packaging/test_native_package.py
python -B packaging/test_native_source.py
python -B packaging/macos/test_macos_package.py
```

需要 Rust 回归时按模块选择测试，使用单个构建任务和测试线程。纯界面模型可脱离渲染栈测试。隔离运行需同时指定绝对路径 `FLOW_ROAMING_DATA_DIR`、`FLOW_LOCAL_DATA_DIR`、`FLOW_DOWNLOAD_DIR`；`FLOW_DATA_DIR` 只覆盖核心数据目录。详见 [贡献指南](CONTRIBUTING.md) 和 [验证说明](docs/validation.md)。

## 许可与第三方组件

流光自有源码使用 [Apache License 2.0](LICENSE)。第三方代码、库、运行组件和许可文件保留各自授权，详见 [第三方声明](THIRD_PARTY_NOTICES.md)。

运行组件由平台下载锁固定版本和 SHA-256。Windows 使用 GPL shared FFmpeg；Mac 使用独立锁定的第三方 FFmpeg 构建。源码仓库不分发这些二进制或大型对应源码归档。发布包含媒体工具的二进制前，应按实际组件补齐许可和对应源码材料；当前覆盖范围及待确认项见 [来源记录](docs/third-party-source-status.md)。

- [使用指南](docs/user-guide.md)
- [架构](docs/architecture.md)
- [仓库结构](docs/repository-layout.md)
- [发布流程](docs/release-checklist.md)
- [问题反馈](https://github.com/desirewnds/liuguang/issues)
