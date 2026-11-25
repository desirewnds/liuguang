# 第三方来源核验记录

来源核验日期：2026-11-15；存储布局更新：2026-11-16。这里记录实际取得的文件与检查结果，不代替公开发布时维护者对实际再分发范围的确认。

当前仅保留合并归档 `engine/sources/open-source-materials.zip`。本文所列源材料文件名及展开目录均指归档中的 `engine/sources/` 成员，不表示本地另有展开副本。外层归档由 `engine/sources-manifest.json` 校验，内部条目由归档内 `source-materials-manifest.json` 校验；归档中的旧文档是采集快照，当前存储布局以本文为准。

## 2.0 原生迁移增量核对

2026-11-16：Iced 原生迁移保持 yt-dlp、FFmpeg、FFprobe 和 Node 二进制不变，移除原生包中的 WebView2。新的 Windows GNU Rust 依赖索引含 287 项，513 份许可文本均已归档；24 份补充上游文件按 Git blob 身份校验，来源和版本记录在 `packaging/upstream-notices/index.json`。许可收集不等于完整法律审查，索引中的 `complete_licensing_review` 仍为 false。

当前唯一独立 copyleft Rust 依赖为 option-ext 0.2.0（MPL-2.0）。现有材料包已包含 `engine/sources/application/option-ext-0.2.0.crate`，7,345 字节；实际条目 SHA-256 与当前 Cargo.lock 一致：`04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d`。归档包含源文件、Cargo 配置、README 和完整 MPL 文本。self_cell 1.3.0 使用其 Apache-2.0 许可选项。

本次仅读取材料包中央目录和相关小条目，没有重压或复制整个归档。下述七个应用 MPL 源包属于 1.x 历史采集范围，不表示原生版仍依赖七个 MPL 包。旧 npm/Tauri 许可不进入原生发行包，新版许可由 packaging/notices 覆盖；媒体引擎的既有来源审查边界继续保留。

## 已核实的运行时

| 组件 | 官方身份验证 |
| --- | --- |
| yt-dlp 2026.10.21 | 已复制二进制与固定 GitHub release `SHA2-256SUMS` 中的 `yt-dlp.exe` 一致 |
| Node.js 24.15.0 | 已复制二进制与官方 `SHASUMS256.txt` 中的 `win-x64/node.exe` 一致 |
| FFmpeg / FFprobe 与 7 个私有 DLL | 从 SHA-256 为 `94dd295461cada31a9e7f79ec0f9ec317d9afcbfe9e60b337679bce2c5a60016` 的官方固定 shared 归档按精确成员路径读取；全部 9 个运行文件另有校验 |

桌面 FFmpeg 使用 FFmpeg-Builds 发布 `autobuild-2026-11-14-12-37` 的 `n8.1.3-35-gf1acb0a266` GPL shared 构建。固定配方提交是 `c9fe5114060e507409f7ee07edfd82efaf25d767`。原始配方归档、官方校验文件、发布元数据、实际 `-version` 配置输出均保留。

1.0.2 从同一发布的 static 变体切换为 shared 变体，保留 GPL 与 version 3 配置，仅新增 `--enable-shared --disable-static`。编译器、版本、库 ABI、16 类功能列表以及 FFmpeg/FFprobe 完整命令帮助核验一致；H.264/AAC 编码、探测、转封装、解码帧摘要回归通过。PE 普通导入与延迟导入的依赖闭包只包含随包的 7 个 FFmpeg DLL 和 Windows 系统库。结果和原始输出保存在 `ffmpeg-shared-verification.json` 与 `ffmpeg-shared-verification/`。

两个 EXE 与 7 个 DLL 共 199,772,160 字节，相比旧两个静态 EXE 的 330,678,272 字节减少 130,906,112 字节（124.84 MiB）。这不依赖压缩方式，不裁剪编解码器、协议或滤镜。硬件加速列表相同仅证明编译入口保留，不代表已在各种显卡上逐项运行；按名称动态加载的可选 GPU/系统驱动仍受用户硬件与系统条件影响。

## 实际归档的源材料

- **134 条直接依赖记录**：构建配方中声明的 Git/SVN 修订以及 FFmpeg 自身源码全部已取得；记录含一些非 Windows 或可选组件，不等于二进制链接了全部条目。
- **52 条嵌套源记录**：递归 Git 子模块以及 shaderc 的固定 DEPS 依赖均已归档；本次子模块检查没有错误。
- **576 个 Cargo 源包**：来自 librsvg 与 rav1e 源码中的锁文件，全部与锁定 SHA-256 一致。包含构建、测试和非当前平台依赖。
- **7 个应用 MPL 源包**：对应本工程锁文件中 cssparser、cssparser-macros、dtoa-short、option-ext、selectors 的相关版本；均按 Cargo.lock 校验。
- Xvid 使用官方 SVN **2204** 版，LAME 使用官方 SVN **7113** 版；目录及文件按固定版本读取，没有用最新源码代替。
- MinGW-w64、libiconv、gnulib、opencore-amr 和 soxr 从配方指定提交获取并归档；gnulib 使用配方声明的官方 GitHub 镜像相同提交。
- GPLv3、FFmpeg 构建配方许可、Node 完整聚合许可、yt-dlp 聚合许可、MPL2 和 GCC Runtime Library Exception 已保留。归档中的各依赖原始许可和版权文件另行汇总到 `engine/licenses/ffmpeg-dependencies/`。

具体条目、来源和摘要分别位于归档中的 `ffmpeg-source-inventory.json`、`ffmpeg-nested-source-report.json`、`ffmpeg-cargo-source-report.json`、`application-source-index.json`。源码归档不包含个人下载、浏览器配置或 Cookies。

## 审查边界

已取得声明源码与已成功重新编译是不同证据。本次没有在 Linux Docker 交叉编译环境中完整重建 FFmpeg，也没有证明再构建产物与上游二进制逐字节相同。

上游配方会获取部分通用构建工具与镜像的可变版本，并在 rav1e 构建时更新 `cc` 构建辅助 crate。归档保留原始配方和媒体库原有锁文件，没有伪造上游最终构建环境或修改后锁文件。公开发布前应确认最终实际启用的依赖范围、构建辅助工具与运行库的适用材料，并在公开位置保存本次源材料。来源报告的 `complete_corresponding_source_verified` 保持 `false`，表示没有把源包下载成功自动升级为完整法律/重建审查结论。

1.x 历史应用依赖采集以 npm/Cargo 包为依据。其测试依赖 stackback 0.0.2 的 npm 包只声明 MIT、未附独立许可全文；这一历史缺口不属于已排除 npm 的原生运行包。新版 Rust 包缺少根目录许可时按包内 VCS 提交追溯官方仓库，不把历史缺失状态错误沿用到新索引。

## Gyan 8.1.2 评估结果

原网页版二进制来自 Gyan `8.1.2-essentials_build`。其官方归档及 SHA-256、README、FFmpeg 8.1.2 源码和 GPLv3 已取得，但公开材料没有完整给出全部静态依赖的确切版本与构建脚本；同版本上游 [issue #246](https://github.com/GyanD/codexffmpeg/issues/246) 也记录了此问题。流光桌面版因此使用上述公开配方构建，原网页版文件未改变。8.1.2 材料只是调查记录，不作为桌面当前 FFmpeg 源码。

## 重建入口

原配方中的 `README.md`、`makeimage.sh`、`build.sh`、`download.sh`、Docker 配置和 `scripts.d/` 已随原归档保留。1.0.2 构建目标为 `win64 gpl-shared 8.1`，实际 FFmpeg 修订为 `f1acb0a266`；旧 1.0.1 为同版本 `win64 gpl 8.1`。重建时必须先将可变的 release/8.1 分支固定到这一修订，并将依赖下载替换为已校验的本地归档；不能直接运行当前最新分支后称其为同一构建。

旧 static 二进制 ZIP、重复源码材料包、运行时下载压缩包及 Git 下载镜像已在清理中删除。旧版 downloads lock、配置输出、源材料及核验记录保留在合并归档中；当前 `engine/downloads.lock.json` 只恢复 shared 变体。本地保留一套展开的当前运行依赖及最新安装包，需要对外发布时再生成运行时和源材料附件。上游每日 release 有保留期限，不能把每日下载地址当永久可用来源。

采集脚本只读取声明和下载来源，不执行上游配方。每次引擎更新都应重新生成上述来源记录，不复用旧版本的审查状态。
