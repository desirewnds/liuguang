# 贡献

先阅读 [README](README.md) 和 [架构](docs/architecture.md)。这是 Rust/Iced 桌面工程，仓库根目录为 `desktop/` 本身。代码、构建和测试不得依赖父目录的网页版、旧 Tauri/React 界面或个人配置；维护桌面版时不修改网页版。

## 提交范围

按 [仓库结构](docs/repository-layout.md) 提交源码、Cargo 锁文件、原生资源清单及许可材料。运行二进制、Cargo 缓存、编译输出、测试临时文件、发布附件和个人数据留在 Git 之外。

从桌面工程根目录运行 `python -B scripts/package-native-source.py --list` 可查看源码白名单及大小，不创建归档。新增文件需同步核对 Git 忽略规则和源码归档白名单。不要使用旧 `package-release.py` 作为原生发行入口，也不要强制添加运行二进制。

## 开发与资源

Windows 构建目标为 x64 GNU，使用 Rust、Python 3.11 或以上版本及所需 GNU/CMake 工具；安装包编译另外需要已有 NSIS 3。Mac 使用 Apple Silicon 或 Intel 原生构建机，所需工具与验证边界见 [Mac 打包说明](packaging/macos/README.md)。本地脚本不自动安装全局工具，不修改系统 PATH。

`scripts/prepare-native-engines.py --list` 只显示依赖选择；实际恢复使用同一脚本，已有准确归档时传 `--offline-dir`。原生入口只选择 yt-dlp、共享 FFmpeg/FFprobe 与 Node，不准备 WebView2。不要因下载或校验失败改用 latest、删除校验值或重新生成信任记录。

`scripts/build-native.ps1` 默认一个 Cargo 任务和低于普通的进程优先级，缓存与临时文件限于工程目录。它先收集原生许可、生成编译资源清单，再构建和审计；`-NoPackage` 不编译安装包，`-Offline` 要求依赖已在项目缓存中。`Cargo.lock` 或许可/资源变化后必须重新准备 `packaging/native-engine-manifest.json` 并重建，不能跳过清单不一致错误。

## 验证边界

先执行与改动有关的小测试，按组串行运行。无需为了文档或样式小改动重复构建整个工程。包装与源码边界检查入口为：

```powershell
python -B packaging/test_native_package.py
python -B packaging/test_native_source.py
```

这些测试使用项目内微型夹具，不下载运行依赖或运行安装器。第一条只有显式提供 `--makensis` 时才编译微型安装模板；生成物仍不会执行。Rust 检查使用一个构建任务、单测试线程和适合改动的测试组；不默认运行压力测试、长时间下载或实机安装验证。完整说明见 [验证范围](docs/validation.md)。

测试数据、配置和视频必须使用独立位置。不得扫描用户浏览器登录、代理端口或下载目录。真实网站验证使用明确提供的链接及主动配置的认证，并记录网络条件。安装、升级、卸载、快捷方式和环境变更应在获准的隔离 Windows 中验证；目前没有完整实机安装生命周期验收，不能用单元测试或 NSIS 编译替代。

三份 Actions 工作流均仅手动触发：`check.yml` 运行轻量 Python 边界检查；`release-check.yml` 在 Windows GNU runner 构建、运行选定小测试并编译安装包；`macos-check.yml` 串行构建 Apple Silicon 和 Intel 应用包。构建流程默认不上传 artifacts，不运行 GUI 或安装器，也不发布 Release。Windows 不签名；Mac 默认 ad hoc 签名，不代表 Developer ID 或公证。只有真实运行结果才能作为 CI 通过证据。

## 行为与界面

界面更改保留暖灰、砖红操作、灰蓝进度的视觉分工，照顾中文、键盘、长标题和高缩放。不要用演示成功状态替代未实现行为。下载选项不得静默丢弃；会改变普通任务协议的选项应明确进入工具工作区。

涉及引擎版本时同步审查下载锁、原生选项目录、能力矩阵、许可和来源材料，并验证失败与恢复路径。复用已经核对的第三方材料，避免仅为新增宽松许可依赖重复制作大型材料包；新增对应源码义务必须单独核对。

问题报告应包含应用/引擎版本、最短复现步骤与期望和实际行为。提供日志前去除 Cookies、代理密码、请求头、用户名和个人路径。不要提交个人数据库、下载文件或认证材料。

## 贡献许可

流光本体使用 [Apache License 2.0](LICENSE)。除明确另行约定外，为合入本项目而主动提交的自有代码、文档和资源贡献按 Apache-2.0 提供，见许可证第 5 条；提交者应有权提供该贡献。第三方代码保留原许可、来源和必要告知，不得将第三方材料统一改写为本项目许可证。

自有源码使用 `SPDX-License-Identifier: Apache-2.0` 标识。引用外部实现时注明来源并保留适用声明；许可范围和依赖边界见 [许可证说明](docs/license-options.md) 与 [第三方告知](THIRD_PARTY_NOTICES.md)。
