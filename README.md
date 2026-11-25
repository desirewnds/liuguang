# 流光 · Liuguang

流光是一款基于 yt-dlp 和 FFmpeg 的视频下载器。粘贴 YouTube 播放列表或哔哩哔哩分 P 视频的链接，就能按原集号挑选想保存的几集。下载单个视频时，也可以直接选择画质、格式和保存位置。

[发行版本](https://github.com/desirewnds/liuguang/releases) · [使用指南](docs/user-guide.md) · [反馈问题](https://github.com/desirewnds/liuguang/issues)

## 获取流光

**Windows 64 位**　安装包内含 yt-dlp、FFmpeg、FFprobe 和 Node.js。发布后，在发行页展开 **Assets（附件）**，下载以 `windows-x64-setup.exe` 结尾的文件并双击安装。[查看发行版本](https://github.com/desirewnds/liuguang/releases)

**macOS**　Apple Silicon 和 Intel 均有源码构建说明，目前尚无经 Mac 实机验证的安装包。[macOS 构建指南](packaging/macos/README.md)

## 第一次下载

1. 点击 **＋ 添加下载**，粘贴链接，点击 **解析链接**。
2. 勾选视频。下载合集时，可以输入 `1-3,7` 再点 **选集**，一次选中第 1、2、3 和 7 集。
3. 选择画质、格式与保存位置。想知道要下载多少数据，可点 **估算所选大小**。源站提供的信息不足时，大小会显示为未知。
4. 点击 **下载所选 N 项**。任务进入下载队列，完成后可在所选目录找到视频。Windows 默认保存到系统“视频”文件夹下的 `流光` 目录。

“最佳画质”会选择源站向当前账号提供的最佳可用格式。想固定分辨率时，可以直接选 1080p 等档位。源站缺少所选分辨率时，默认会提示失败，可在设置里允许自动降低分辨率。

要单独保存音轨，在格式中选择音频即可。字幕语言、代理、Cookies 和文件名模板可在 **高级设置** 中查找，常用组合可以保存为预设。喜欢用终端的用户也可以通过安装目录中的 `flow.cmd` 直接调用 yt-dlp。

## 开发与贡献

流光使用 Rust 和 Iced 构建桌面界面。要从源码运行或制作安装包，请按平台的构建说明准备工具链和下载组件。[Windows 构建指南](packaging/README.md) · [macOS 构建指南](packaging/macos/README.md) · [贡献指南](CONTRIBUTING.md)

报告下载问题时，请附上流光版本、可公开的示例链接和错误提示。提交前请移除 Cookies、代理密码等私人信息。[提交问题](https://github.com/desirewnds/liuguang/issues/new/choose)

流光自有代码采用 [Apache License 2.0](LICENSE)。yt-dlp、FFmpeg 等第三方组件各自遵循原许可证。[第三方声明](THIRD_PARTY_NOTICES.md)
