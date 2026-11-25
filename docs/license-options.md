# 项目许可证

流光本体采用 **Apache License 2.0**。仓库根目录的 [LICENSE](../LICENSE) 为完整英文原文；维护者已于 2026-11-25 选定该许可证。项目由 [desirewnds](https://github.com/desirewnds) 维护，源码仓库为 [desirewnds/liuguang](https://github.com/desirewnds/liuguang)。

## 适用范围

除文件自身另有声明外，本许可证覆盖本项目的自有代码、构建和打包脚本、文档及原创资源。自有源码以 `SPDX-License-Identifier: Apache-2.0` 标识，Rust 包通过 workspace 继承许可和仓库元数据。

Apache-2.0 允许使用、修改及再分发，也允许按其条件用于商业或闭源衍生项目。再分发时需遵守原许可证中的许可副本、改动说明及告知保留要求；其专利、商标和免责条款以 [Apache 官方正文](https://www.apache.org/licenses/LICENSE-2.0) 为准。项目采用标准原文，没有增加“禁止商用”等额外限制。

## 第三方材料

依赖库、下载插件、yt-dlp 来源的参数目录和第三方许可/来源材料继续适用各自声明，不因项目本体的许可证选择而改为 Apache-2.0。`engine/` 中的第三方代码及材料、`packaging/notices/` 和 `packaging/upstream-notices/` 不应被批量重新授权。

当前使用的 FFmpeg 为 GPL 构建。通过独立进程调用组件，不自动免除实际分发该组件时需要承担的义务。第三方许可索引和来源记录仍按真实核验范围保留；本体许可证的确定不代表第三方来源与二进制分发审查已全部完成。详见 [第三方告知](../THIRD_PARTY_NOTICES.md)、[来源核验记录](third-party-source-status.md) 和 [Mac 打包说明](../packaging/macos/README.md)。

## 贡献

默认贡献条款与项目许可证一致，详见 [贡献指南](../CONTRIBUTING.md)。Apache-2.0 不授予额外的项目名称或商标使用权。版权仍由各权利人持有，不使用虚构的统一版权声明替代第三方原有声明。
