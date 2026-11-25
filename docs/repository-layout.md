# 原生仓库结构

[desirewnds/liuguang](https://github.com/desirewnds/liuguang) 是独立的 Rust/Iced 桌面版源码仓库，应用本体采用 Apache-2.0。仓库根目录直接包含 Cargo workspace；网页版不进入这个仓库。源码、运行依赖、构建缓存和交付附件按用途分开。

## 提交到源码仓库

    crates/flow-gui/             Iced 界面、图像、纯状态模型
    crates/flow-app/             应用协调和有界事件
    crates/flow-core/            下载和数据核心
    crates/flow-platform/        Windows/Mac 平台、私有路径和生命周期
    crates/flow-cli/             命令行入口
    Cargo.toml / Cargo.lock      统一依赖锁与构建配置
    packaging/                  独立安装器、小图标、原生清单和许可
    scripts/                    原生依赖准备、校验及构建脚本
    tests/                      核心、平台、包装与合成媒体测试源码
    docs/                       使用、架构和实际验证记录
    .github/                    手动工作流和协作模板
    engine/catalog/             固定版本参数目录
    engine/plugins/             下载插件源码
    engine/licenses/            引擎许可及来源材料索引
    engine/*.json               下载锁及历史来源记录
    packaging/flow.cmd          Windows CLI 启动模板
    README.md / CONTRIBUTING.md / LICENSE / THIRD_PARTY_NOTICES.md

原生源码包排除旧 frontend/、src-tauri/ 和旧应用依赖许可，收录 packaging/notices/ 与其引用的上游文本。应用代码使用根目录 LICENSE 中的 Apache-2.0；第三方代码、工具及许可文本继续遵循各自许可证。各平台验证范围见 README 和验证记录。

## 不提交的内容

| 内容 | 用途 |
| --- | --- |
| engine 的 EXE/DLL、Mac Mach-O/dylib | 随包运行依赖，按各平台下载锁恢复 |
| engine/webview2/、WebView2Loader.dll | 旧版浏览器运行资源，已从桌面目录移除 |
| .build/、target/ | 项目内依赖缓存、编译与临时文件 |
| test-artifacts/ | 明确隔离的测试输出 |
| release/ | 安装包及发布附件 |
| 用户配置、Cookies、数据库、日志、视频 | 私有数据，不是源码 |
| engine/sources/open-source-materials.zip | 一份第三方材料，单独发布，不装入应用 |

源码仓库不提供根目录 EXE；开发者先恢复锁定依赖，再构建 GUI 和 CLI。原生打包流程不使用旧界面。运行时目录与源码体积必须分别统计，开发缓存不是安装包内容。

只读查看实际源码范围：

    python -B scripts/package-native-source.py --list

归档使用明确白名单并校验内嵌资源与依赖锁。只读列表不检查每个文本是否含有凭据；首次提交仍需检查内容和暂存区。Git 忽略不会移除此前已跟踪的文件，不应强制添加二进制绕过边界。

原生依赖入口是 prepare-native-engines.py，只恢复 yt-dlp、FFmpeg/FFprobe 及 Node。它不获取 WebView2。实际安装器由 build-native.ps1 和 package-native.py 生成，源码包由 package-native-source.py 生成；历史 Tauri 脚本不作为新版本入口。

Mac 使用独立的 `packaging/macos/downloads.lock.json` 和 `scripts/build-macos.py`，按 Apple Silicon/Intel 分开构建。依赖与中间文件均写入 `.build/macos/`，不会替换 Windows 的 `engine/` 或已有安装包。源码归档包含两个平台的源码与构建说明，运行依赖不打进源码 ZIP。

## 体积记录

旧 1.0.2 安装包为 433.77 MiB，入包未压缩文件为 1,020.56 MiB。其中 WebView2 约 678.12 MiB；原生迁移移除这部分。保留的 FFmpeg、Node 和 yt-dlp 约 295 MiB，不能用 Iced 主程序大小代表完整离线包。

Windows 2.0 的已构建载荷为 331.80 MiB，安装包为 135.07 MiB。源码白名单约 15 MiB，最终数字以实际生成的清单为准。维护者保存的第三方材料约 1.42 GiB，不进入 Git，也不装入应用；媒体工具二进制公开发行时仍需提供匹配的来源材料。MPL 依赖 option-ext 的对应 crate 与 Cargo 锁哈希已有核对记录，完整媒体来源覆盖仍有待确认。具体边界见 [2.0 验证记录](verification-2.0.0.md) 和 [来源核验](third-party-source-status.md)。

`target/`、`.build/` 和 `test-artifacts/` 都可以重新生成，不提交到源码仓库。清除开发缓存后，按 README 重新执行 `cargo fetch` 并构建即可；使用完整发行包不需要这些开发缓存。

测试结束清理已确认归属的临时目录；正式发布才生成所需附件，不日常保留多套压缩副本。
