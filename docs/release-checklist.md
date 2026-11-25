# 发布检查

本地构建、可分发安装包、开源仓库及对外签名发布是不同状态。以下检查只针对 Rust/Iced 原生桌面版；网页版及旧交付基线保持独立。当前没有完成完整的隔离 Windows 安装、升级、卸载验收，不能将构建成功写成已完成这些验收。

## 手动 Actions

[desirewnds/liuguang](https://github.com/desirewnds/liuguang) 的仓库根目录就是桌面工程，可以手动运行三份工作流：

- `.github/workflows/check.yml`：小型 Python 包装/源码边界检查，以及只读原生依赖列表；不编译 Rust、不下载引擎、不生成安装包或 artifacts。
- `.github/workflows/release-check.yml`：一次性 Windows GNU runner 恢复锁定媒体依赖、准备 GNU 工具、以单任务构建原生程序、分组运行小测试并编译未签名 NSIS 安装包；不运行 GUI 或安装器。
- `.github/workflows/macos-check.yml`：Apple Silicon 与 Intel 原生 Mac runner 串行恢复各自锁定依赖、构建 `.app` 和 DMG；不启动 GUI、不安装应用，也不做升级或卸载验收。具体签名与许可边界见 [Mac 打包说明](../packaging/macos/README.md)。

三个入口都不自动创建 Release、修改标签，也不会因 push 自动触发。Windows 构建不签名；Mac 默认只做 ad hoc 签名，不代表 Developer ID 签名或公证。原生构建工作流的 `upload_artifacts` 默认关闭；主动开启后才保存相应成品及审计材料，不另行上传应用源码。artifacts 设置保存 14 天，并受仓库策略限制，不能视为永久发布存储。

默认使用 `engine/downloads.lock.json` 固定 URL 与校验值恢复 yt-dlp、共享 FFmpeg/FFprobe 和 Node，共 11 个输出，不选择 WebView2。也可通过 `engine_artifact_run_id` 指定同仓库保留的 `liuguang-native-locked-archives`，恢复后仍按锁校验。上游失效且没有保留归档时必须停止，由维护者提供准确的离线归档或审查资源更新，不自动替换为 latest。

工作流配置尚未实际运行，不能声称 Actions 已通过。CI 成功也只证明其列出的构建、选定测试和静态审计；不证明界面易用性、实际安装行为或完整许可审查。

## 本地构建与打包

以下命令以桌面工程根目录为工作目录。先按 README 准备已有 Windows GNU 工具和项目 Cargo 缓存，再依次检查：

- 用 `python -B scripts/prepare-native-engines.py --list` 核对依赖选择。实际恢复用同一脚本；已有准确归档时传 `--offline-dir`，只校验现有文件时传 `--verify-only`。
- 维护 `Cargo.lock`、`packaging/notices/` 和 `packaging/native-engine-manifest.json` 的一致性。原生清单从 canonical 资源筛选并叠加新版许可，不能要求安装 WebView2，也不能带回旧 Tauri/npm 运行依赖。
- 用 `./scripts/build-native.ps1 -NoPackage -Offline -Jobs 1` 构建及审计载荷；该入口默认低于普通优先级，缓存和临时文件限制在工程内。`-Offline` 仅适用于依赖已经准备好的情况；需要显式工具目录时传 `-ToolDirectories`。
- 按改动范围执行 `python -B packaging/test_native_package.py`、`python -B packaging/test_native_source.py` 和相关 Rust 小测试组。Rust 使用一个构建任务、单测试线程，不默认运行压力测试或长时间真实下载。
- 用明确的已有编译器路径执行 `python -B scripts/package-native.py --compile --makensis <makensis.exe绝对路径>`。编译前必须通过载荷哈希、编译清单一致性和 PE 导入依赖审计。脚本不运行安装器，也不覆盖已有同版本 release 文件。
- 核对实际载荷包含 GUI、CLI、`flow.cmd`、媒体工具、Node、选项目录与许可；不含 WebView2、WebView2Loader、开发缓存或个人数据。历史下载锁可以保留旧浏览器获取元数据，但原生恢复入口与实际载荷不得包含浏览器运行文件。
- 记录实际工具链、平台、安装包大小、载荷大小与 SHA-256。旧安装包保留为基线，新包验证完成前不删除。源码能独立准备和构建，不能引用父目录网页版、旧 `src-tauri` 或 `frontend`。

构建脚本不安装全局工具，不更改系统 PATH。CPU、内存、磁盘压力较大的构建和压包应串行安排；文档或小改动不反复触发完整验证。产物已经通过相关检查后，只在新改动或明确失败需要时重跑。

## 安装生命周期验收

安装、升级和卸载的实机结果目前保持“未测”。以下是后续隔离 Windows 验收清单；不在普通开发电脑上试装：

- 新装后 GUI/CLI 能启动，主界面无命令窗口；单实例、托盘、退出与子进程收尾符合约定。
- 从已交付旧版升级到原生版时，设置、预设和历史按规则保留，旧浏览器缓存仅在所有权明确且不与视频重叠时清理。
- 安装中断后，同一个安装包能在原位置安全重试；同名外来文件、目录链接、被改动文件和活动任务会阻止危险替换。记录升级失败的实际恢复步骤，不宣称整个安装具有自动事务回滚。
- 普通卸载清理应用自己的配置、缓存和临时文件；`/KEEPDATA` 保留设置等持久状态，`/UPDATE` 不清理用户状态。视频始终独立保留。
- 覆盖自定义数据/视频位置、未知同名文件、缺失文件、文件占用、目录链接及异常退出。核对每次失败后视频和所有权证明仍可用。
- 对比安装前后的 PATH、服务、共享运行时与相关注册表项目。应用只使用私有运行组件和必要的当前用户应用注册，不安装全局浏览器运行时。

未执行项必须在验证报告中保留“未测”，不能用纯函数测试、静态导入检查或 NSIS 编译结果替代。当前验证状态以 `docs/validation.md` 和 `docs/native-packaging-verification.md` 为准。

## 开源与公开发布

- 以桌面工程为仓库根目录，遵守 `docs/repository-layout.md`。先用 `python -B scripts/package-native-source.py --list` 预览白名单与大小；确定发布材料后才显式运行 `--archive`。不要上传整个本地文件夹。
- 源码归档使用原生入口，不使用旧 Tauri 的 `package-release.py`。归档须含 Rust workspace、统一 Cargo.lock、必要 catalog/plugin 源码、打包脚本、手动工作流和原生许可；排除运行二进制、旧界面、缓存、视频、Cookies、代理密码、日志及数据库。
- 应用源码包与第三方对应源码材料是不同附件。保留并复用已核对版本的那一份 `engine/sources/open-source-materials.zip`；有新增对应源码义务时核对具体包、版本和 Cargo 校验值，不因新增 MIT 等依赖就盲目重建大型材料包。
- FFmpeg 构建配方、实际启用组件、直接与嵌套依赖的材料和取得方式需保持可审阅；仅有 FFmpeg 上游 tarball 不代表全部来源材料。当前范围见 `docs/third-party-source-status.md` 与第三方告知文件。
- 对外发布时提供相应许可、版权告知及可取得的对应源码材料，保存匹配版本；上游每日构建和短期 Actions artifacts 不是长期来源保证。许可文本已收集不等于完整法律审查通过。
- 应用本体采用 Apache-2.0，保留根目录 LICENSE 和第三方声明。二进制发布仍需单独确认来源材料、发布附件与签名状态；未签名包如实标明，未设定真实更新服务和签名时不声称应用自动更新可用。
- 发布说明只列真实支持、测试的平台和行为，单独列明仍未完成的实机安装生命周期验收。

## 资源更新

资源更新必须审查官方来源、锁定校验值、选项目录差异、JavaScript 兼容性、媒体行为及许可/源码材料。保留可回退的已验证版本，避免活动下载期间替换运行文件。

更新下载锁及生成完整性清单是有审查依据的维护操作，不是遇到校验失败时的默认修复。Cargo 依赖或许可材料变化后重新收集 notices、冻结原生编译清单并重建；打包检查不得被跳过。材料采集不执行上游构建脚本，缺口保持真实状态。
