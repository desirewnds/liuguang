# SPDX-License-Identifier: Apache-2.0
"""Add maintained Chinese explanations without changing yt-dlp option semantics."""
from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[1]
EXPLANATIONS = r'''
--help|显示当前引擎的完整帮助后退出。
--version|显示实际运行的 yt-dlp 版本后退出。
--update|检查并更新原生命令行引擎；桌面管理的版本应通过设置中的引擎管理更新。
--no-update|本次运行不检查引擎更新，这是 yt-dlp 的默认行为。
--update-to|升级或降级到 CHANNEL@TAG；支持 stable、nightly、master 或明确指定的仓库。
--ignore-errors|忽略下载和后处理错误。即使合并或转码失败也可能返回成功，使用前应确认其影响。
--no-abort-on-error|某个视频失败后继续处理后续视频，这是引擎默认行为。
--abort-on-error|任一视频发生错误时停止本次引擎命令中的后续视频。
--list-extractors|列出引擎支持的全部网站提取器后退出。
--extractor-descriptions|列出网站提取器的说明后退出。
--use-extractors|用逗号分隔提取器名称或正则表达式；名称前加减号可排除，例如 default,-generic。
--default-search|为非完整链接指定搜索前缀。默认 fixup_error 只尝试修正链接，不自动搜索。
--ignore-config|停止自动读取额外配置文件；明确通过 config-locations 指定的文件除外。
--no-config-locations|不读取自定义配置文件；放在配置文件中时会清除该文件中此前声明的位置。
--config-locations|指定配置文件或目录，可添加多个；减号表示从标准输入读取。
--plugin-dirs|添加插件目录，可重复；default 表示引擎默认插件目录。只加载自己信任的插件。
--no-plugin-dirs|清除此前指定及默认的插件搜索目录。
--js-runtimes|启用 JavaScript 运行时，格式为名称[:路径]；支持 deno、node、quickjs、bun。
--no-js-runtimes|清除已启用的 JavaScript 运行时；需要时再用 js-runtimes 指定。
--remote-components|允许按需获取指定远程组件，如 ejs:npm 或 ejs:github。官方可执行文件通常已包含所需组件。
--no-remote-components|禁止下载任何远程组件，并清除此前的允许列表。
--flat-playlist|只读取合集目录而不深入解析每个视频；部分标题、格式及大小可能缺失。
--no-flat-playlist|完整解析合集中的视频，这是引擎默认行为。
--live-from-start|尝试从直播开始处下载；属于实验功能，支持情况取决于网站。
--no-live-from-start|从当前直播位置开始下载，这是引擎默认行为。
--wait-for-video|等待预约直播上线，填入重试间隔秒数或范围，例如 30-60。
--no-wait-for-video|预约直播尚未开始时不等待，这是默认行为。
--mark-watched|向支持的网站标记已观看；模拟运行时也可能发送标记，需要有效登录状态。
--no-mark-watched|不向网站发送已观看标记，这是默认行为。
--color|控制原生输出颜色；支持 auto、always、never 等，可用 stdout: 或 stderr: 指定输出流。
--compat-options|启用与 youtube-dl 或旧版 yt-dlp 行为兼容的选项；多个名称用逗号分隔。
--alias|定义参数别名及展开模板，可重复；模板采用 Python 字符串格式语法，避免循环别名。
--preset-alias|应用引擎内置参数组合，例如 mp3、aac、mp4、mkv、sleep，可重复。
--list-impersonate-targets|列出此引擎构建实际可用的浏览器模拟目标。
--enable-file-urls|允许读取 file:// 本地文件地址。引擎默认禁止此类地址。
--geo-verification-proxy|只为部分站点的地区验证指定代理；实际下载仍使用常规代理设置。
--xff|设置 X-Forwarded-For 策略：default、never、CIDR 网段或两位国家代码；不保证能改变网站地区限制。
--playlist-items|按原始集号选择，可用 1,3,7:10 或 1:20:2；负数从末尾计数，步长可为负数。
--match-filters|按元数据筛选，如 !is_live 或 duration<600；同一条中的 & 表示同时满足，多条规则之间表示或。
--no-match-filters|清除通用视频筛选规则。
--break-match-filters|语法与 match-filters 相同，但遇到不满足条件的视频时停止本次命令。
--no-break-match-filters|清除遇到不匹配即停止的筛选规则。
--no-playlist|链接同时包含视频和合集时只下载当前视频。
--yes-playlist|链接同时包含视频和合集时下载合集。
--age-limit|只下载适合指定年龄观看的视频；不是登录或年龄验证的替代方式。
--no-download-archive|不使用已下载记录文件，这是引擎默认行为。
--max-downloads|下载指定数量文件后停止本次引擎命令。
--break-on-existing|遇到已存在于下载记录文件中的条目时停止本次命令。
--no-break-on-existing|遇到下载记录中已有条目时跳过并继续，这是默认行为。
--break-per-input|对每个输入链接分别重置下载上限、遇到已下载即停止和自动编号等规则。
--no-break-per-input|下载上限及遇到已下载即停止等规则作用于整个命令输入队列。
--skip-playlist-after-errors|某合集失败达到指定次数后跳过该合集剩余条目。
--skip-unavailable-fragments|跳过无法下载的 DASH、HLS、ISM 分片；可能产生内容不完整的文件，这是引擎默认行为。
--abort-on-unavailable-fragments|任何分片无法取得时中止该视频下载。
--keep-fragments|下载结束后保留分片文件，便于排查或后续处理，会增加磁盘占用。
--no-keep-fragments|下载结束后删除分片文件，这是默认行为。
--resize-buffer|从 buffer-size 开始自动调整下载缓冲区，这是默认行为。
--no-resize-buffer|保持指定下载缓冲区大小，不自动调整。
--playlist-random|随机排列本次命令的合集下载顺序。
--lazy-playlist|目录条目到达后立即处理；无法预先知道总数，并禁用随机与逆序合集处理。
--no-lazy-playlist|等完整目录解析后再处理视频，这是默认行为。
--hls-use-mpegts|HLS 下载使用 MPEG-TS 中间容器，便于边下边播及中断恢复；直播默认启用。
--no-hls-use-mpegts|HLS 下载不使用 MPEG-TS 中间容器，非直播默认采用此行为。
--downloader|指定外部下载器名称或路径，可加协议前缀，如 dash,m3u8:native；需要自行提供对应可执行文件。
--downloader-args|为外部下载器传入参数，格式为下载器:参数；不同下载器可分别添加。
--batch-file|从文件读取链接，每行一条；减号表示标准输入，#、;、] 开头的行作为注释。
--no-batch-file|不从批量文件读取链接。
--output-na-placeholder|输出文件名模板字段缺失时使用的占位文字，默认 NA。
--restrict-filenames|文件名限制为 ASCII 字符，同时避免空格与 &。
--no-restrict-filenames|允许中文等 Unicode 字符、空格及 &，这是默认行为。
--windows-filenames|强制生成符合 Windows 文件命名规则的名称。
--no-windows-filenames|仅执行最低限度的文件名清理；生成的名称不一定可在 Windows 保存。
--no-overwrites|保留所有已有文件，不覆盖视频及附属文件。
--no-force-overwrites|保留已有视频，但可覆盖相关附属文件；这是原生引擎默认值，流光默认更严格地保留全部文件。
--no-continue|不续传已下载的分片；非分片文件从头重新下载。
--part|下载期间使用 .part 临时文件，完成后再重命名，这是默认行为。
--no-part|直接写入最终文件名，不使用 .part 临时文件。
--mtime|尝试使用服务器 Last-Modified 时间设置文件修改时间。
--no-mtime|不使用服务器时间修改文件时间，这是默认行为。
--write-description|同时保存视频说明文字为 .description 文件。
--no-write-description|不保存视频说明文字文件。
--no-write-info-json|不另外保存视频元数据 JSON 文件。
--write-playlist-metafiles|保存视频元数据时一并保存合集元数据，这是默认行为。
--no-write-playlist-metafiles|只保存视频元数据，不生成合集元数据文件。
--clean-info-json|从元数据 JSON 中移除部分内部字段，例如内部文件名，这是默认行为。
--no-clean-info-json|元数据 JSON 保留全部字段；其中可能包含本地路径及临时媒体地址。
--no-write-comments|通常不额外读取评论；部分提取器能快速取得的评论仍可能出现。
--load-info-json|从之前保存的元数据 JSON 读取下载信息；临时媒体地址可能已经过期。
--no-cookies|不从文件读写 Cookies。
--no-cookies-from-browser|不读取浏览器 Cookies。
--cache-dir|指定引擎缓存目录，保存客户端标识、签名等可重用信息。
--no-cache-dir|禁用引擎磁盘缓存。
--rm-cache-dir|删除引擎缓存目录中的全部缓存文件，不删除下载的视频。
--write-thumbnail|保存视频封面图片。
--no-write-thumbnail|不保存封面图片。
--write-all-thumbnails|保存所有可用尺寸与格式的封面图片。
--list-thumbnails|列出视频可用封面，默认只模拟而不下载视频。
--write-link|按当前系统保存指向原视频网页的快捷方式。
--write-url-link|保存 Windows .url 网页快捷方式。
--write-webloc-link|保存 macOS .webloc 网页快捷方式。
--write-desktop-link|保存 Linux .desktop 网页快捷方式。
--quiet|减少原生输出；同时启用 verbose 时调试日志写入标准错误流。
--no-quiet|恢复正常原生输出。
--no-warnings|隐藏引擎警告，不改变实际下载行为。
--simulate|仅解析与模拟，不下载视频，也不写入下载产物。
--no-simulate|即使使用查询或打印选项也实际执行下载。
--ignore-no-formats-error|无可下载格式时仍尝试返回元数据，适合元数据查询，不会让不可用的视频变得可下载。
--no-ignore-no-formats-error|找不到可下载格式时返回错误，这是默认行为。
--skip-download|跳过视频下载，只处理被要求的字幕、封面或元数据等附属文件。
--print|打印字段或模板，可用阶段:模板指定时机；通常同时启用安静与模拟模式。
--print-to-file|将模板结果追加到指定文件；需要模板和文件名两个参数，可重复。
--dump-json|每个视频输出一行 JSON 元数据，默认只模拟。
--dump-single-json|每个输入链接输出一行 JSON；合集包含在同一个 JSON 对象中，默认只模拟。
--force-write-archive|即使模拟运行，只要没有错误也写入已下载记录；此后实际下载可能被记录跳过。
--newline|将每次进度输出为新行，便于日志处理。
--no-progress|关闭引擎进度条输出。
--progress|即使处于安静模式也输出进度条。
--console-title|把下载进度显示在原生命令行窗口标题中。
--progress-template|自定义下载或后处理进度模板；视频信息在 info 下，进度字段在 progress 下。
--progress-delta|两次进度输出之间的最小间隔，单位为秒。
--verbose|输出引擎版本、参数及详细调试信息。
--dump-pages|将下载的中间网页以 Base64 输出，日志量很大且可能包含敏感信息。
--write-pages|将提取过程中的中间网页保存到当前工作目录，用于排查网站解析问题。
--print-traffic|输出 HTTP 请求与响应详情；其中可能包含认证信息，分享前需逐项检查。
--encoding|强制使用指定字符编码，属于实验功能。
--legacy-server-connect|允许连接不支持 RFC 5746 安全重协商的旧 HTTPS 服务器。
--prefer-insecure|尝试通过未加密连接读取视频信息，仅在确有需要时使用。
--bidi-workaround|处理不支持双向文字的终端；需额外提供 bidiv 或 fribidi 程序。
--format-sort-reset|清除此前自定义的格式排序，恢复引擎默认排序。
--format-sort-force|让自定义格式排序优先于引擎通常保留的优先字段。
--no-format-sort-force|保留引擎部分内置优先字段，这是默认行为。
--video-multistreams|允许将多个视频流合并到同一文件，需要兼容的容器和 FFmpeg。
--no-video-multistreams|每个输出文件只保留一个视频流，这是默认行为。
--audio-multistreams|允许将多个音轨合并到同一文件，需要兼容的容器和 FFmpeg。
--no-audio-multistreams|每个输出文件只保留一个音频流，这是默认行为。
--prefer-free-formats|同等画质优先自由容器；若同时按 ext 排序，可能优先容器而改变最终画质。
--no-prefer-free-formats|不为自由容器设置额外优先级，这是默认行为。
--check-formats|选择格式前检查其是否实际可下载，会增加请求和等待时间。
--check-all-formats|检查所有格式的可下载性，会增加请求量与解析耗时。
--no-check-formats|跳过额外的格式可下载性检查。
--list-formats|列出实际可用格式、分辨率及编码，默认只模拟。
--no-write-subs|不保存人工字幕文件。
--no-write-auto-subs|不保存自动生成字幕文件。
--list-subs|列出视频可用的字幕语言与格式，默认只模拟。
--netrc|从明确配置的 .netrc 认证文件读取账户信息。
--netrc-location|指定 .netrc 文件或目录；不指定时引擎使用其原生默认位置。
--netrc-cmd|运行指定命令取得提取器凭据。这会执行外部程序，只使用自己信任的命令。
--ap-mso|指定 Adobe Pass 电视服务提供商标识，可通过 ap-list-mso 查询。
--ap-username|电视服务提供商账户的登录名。
--ap-password|电视服务提供商账户密码；原生引擎未提供时可能交互询问。
--ap-list-mso|列出 Adobe Pass 支持的电视服务提供商。
--client-certificate|指定 PEM 格式的 TLS 客户端证书，可同时包含私钥。
--client-certificate-key|指定 TLS 客户端证书的私钥文件。
--client-certificate-password|客户端证书私钥的解密密码。
--no-keep-video|后处理成功后删除中间视频文件，这是默认行为。
--post-overwrites|允许覆盖后处理产生的文件，这是原生引擎默认行为。
--no-post-overwrites|保留已存在的后处理输出文件。
--no-embed-subs|不把字幕嵌入视频容器。
--no-embed-thumbnail|不把封面嵌入输出文件。
--no-embed-metadata|不向输出文件添加元数据。
--no-embed-chapters|不向输出文件添加章节标记。
--embed-info-json|将元数据 JSON 作为附件嵌入 MKV 或 MKA 文件。
--no-embed-info-json|不向输出文件附加元数据 JSON。
--xattrs|使用 Dublin Core 与 XDG 规范向文件扩展属性写入元数据；支持情况取决于文件系统。
--concat-playlist|将同一合集视频拼接为一个文件；音视频编码和流数量须一致。可选 never、always、multi_video。
--fixup|修复已知封装问题：never 不修复，warn 仅警告，detect_or_warn 自动检测，force 强制尝试。
--ffmpeg-location|指定 FFmpeg 可执行文件或其目录；桌面版已包含独立 FFmpeg。
--no-exec|清除此前指定的下载后执行命令。
--convert-thumbnails|转换封面格式，支持 jpg、png、webp；none 表示不转换，可用源格式>目标格式规则。
--no-split-chapters|不按章节拆分输出文件。
--remove-chapters|按标题正则表达式或时间范围移除片段，可添加多条规则；需要 FFmpeg。
--no-remove-chapters|不移除章节片段。
--no-force-keyframes-at-cuts|剪辑时不强制插入关键帧，避免因此产生重新编码。
--use-postprocessor|启用插件后处理器，格式为名称:参数名=值;when=阶段；需先安装对应可信插件。
--sponsorblock-chapter-title|定义 SponsorBlock 章节标题模板，可用 category_names、start_time、end_time 等字段。
--no-sponsorblock|关闭 SponsorBlock 标记和片段移除。
--sponsorblock-api|指定 SponsorBlock API 地址，默认 https://sponsor.ajay.app。
--allow-dynamic-mpd|允许处理动态 DASH 清单，这是引擎默认行为。
--ignore-dynamic-mpd|忽略动态 DASH 清单。
--hls-split-discontinuity|遇到 HLS discontinuity 标记时将播放列表分为不同格式，例如广告插入点。
--no-hls-split-discontinuity|不在 HLS discontinuity 标记处拆分格式，这是默认行为。
'''
descriptions = dict(line.split('|', 1) for line in EXPLANATIONS.strip().splitlines())
catalog_path = ROOT / 'engine/catalog/options.json'
catalog = json.loads(catalog_path.read_text(encoding='utf-8'))
for group in catalog['groups']:
    for option in group['options']:
        name = option['name']
        if name in descriptions:
            option['description'] = descriptions[name]
        elif not option.get('description'):
            option['description'] = f"{option['label']}。此项为引擎保留的历史兼容参数；当前行为及替代选项请查看下方原生帮助或工具中的完整帮助。"
        option['scope'] = '原生命令' if option.get('command_only') else '本次下载草稿；保存默认后影响后续新任务'
        option['engine_version'] = catalog['version']
        option['default_behavior'] = '未设置时由引擎默认行为及流光常用设置决定；未设置不等于传入否定参数。'
catalog_path.write_text(json.dumps(catalog, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
print(f"Chinese explanations available for {sum(len(g['options']) for g in catalog['groups'])} options")
