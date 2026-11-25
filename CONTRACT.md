# Desktop integration contract

Only `desktop/` may be changed. The web app, its data and its credentials remain independent.

Native `flow_app::Service::dispatch` takes an operation name and JSON payload and resolves to JSON or a human-readable error. `wait_events`/`drain_events` provide bounded `{kind: string, data: any}` events without a WebView or local HTTP service. Core API: `Core::new(data_dir: PathBuf, engine_dir: PathBuf, emit: Arc<dyn Fn(Value)+Send+Sync>) -> Result<Arc<Core>, String>`, `async dispatch(self: &Arc<Self>, op: &str, payload: Value) -> Result<Value,String>`, and synchronous `shutdown()`.

## Configuration

Settings fields: `directory`; `quality` (`best`, `2160`, `1440`, `1080`, `720`, `480`); `format` (`auto`, `mp4`, `mkv`, `webm`, `mp3`, `m4a`, `flac`, `wav`, `opus`); `concurrency` (1–8, default 2); `proxy_mode` (`system`, `direct`, `custom`); `proxy`; `cookie_file`; `cookie_browser`; boolean `subtitles`, `thumbnail`, `notifications`, `close_to_tray`, `auto_fallback` (default false); `theme` (`system`, `light`, `dark`); `advanced`; `raw_args`. Missing fields inherit saved defaults.

`advanced` maps exact catalog option names to booleans, strings, string arrays, or string[][] for repeated multi-argument options. `false`/`null` disables an override; it does not enable the corresponding negative option. `raw_args` is a JSON string-array text or quoted CLI text. Windows backslashes are preserved. Managed execution rejects command-only options, ambiguous raw/structured duplicates, and conflicting positive/negative flags. Native command tools accept independent engine behavior. Format intent and network/authentication settings are shared by preview and download; preview does not execute download/postprocessing file actions.

## Core operations

| Operation | Payload | Result |
| --- | --- | --- |
| `bootstrap` | `{}` | `{settings,tasks:Task[],profiles:Profile[],engine:Engine,catalog:{version,groups},queue:{paused:boolean},queue_paused:boolean}` |
| `save_settings` | `{settings}` | Normalized settings; persists defaults and emits `settings` |
| `validate_settings` | `{settings,context?:'download'\|'preview'\|'command'}` | Normalized validated settings, no persistence; context defaults to `download` |
| `profiles_save` | `{name,settings}` | `Profile[]`; same-name preset replaced |
| `profiles_delete` | `{id}` | `Profile[]` |
| `preview` | `{url,mode:'auto'\|'video'\|'playlist',settings,request_id?,refresh?:boolean}` | `Preview` |
| `estimate` | `{url,settings,request_id?,refresh?:boolean}` | `{size:number\|null,approximate:boolean,disk_estimate:number\|null,disk_estimate_approximate:true,format_label:string,formats:Format[]}` |
| `cancel_preview` | `{request_id}` | `null`; applies to preview or estimate |
| `enqueue` | `{entries:Entry[],settings,batch_title?,batch_id?,batch_total?,collection_id?,request_id?}` | `Task[]`; atomic batch, repeated request ID does not create duplicates in the current core session |
| `task_action` | `{ids:string[],action:'cancel'\|'retry'\|'retry_current'\|'remove'}` | `null` |
| `list_tasks` | `{}` | `Task[]` |
| `queue_control` | `{paused:boolean}` | `{paused:boolean}`; persisted, affects starting later tasks, running tasks continue |
| `queue_state` | `{}` | `{paused:boolean}` |
| `queue_pause` | `{paused:boolean}` | Alias of `queue_control` |
| `logs` | `{id}` | `{lines:string[],output:string}`; last 500 masked diagnostic lines, plus native UTF-8 text stdout when present; line endings normalized |
| `run_command` | `{args:string,stdin?:string,settings}` | Queued native `Task`; stdin limited to 16 MiB; use batch file for larger input |
| `engine_info` | `{}` | `Engine` |
| `engine_update` / `engine_rollback` | `{}` | `{message:string,engine:Engine}` or rejects with actual failure |
| `engine_verify` | `{}` | `{ok:boolean,files:{path:string,ok:boolean,error:string\|null}[],active_engine:string,active_is_bundled:boolean,message:string}` |
| `diagnostics` | `{}` | `{processes:{task_id:string,pid:number}[],active_tasks:number,queue_paused:boolean}`; only this core's registered processes |

Preview/estimate results cache for 60 seconds, keyed by URL, mode and effective settings; `refresh:true` bypasses cache. Consumers must use distinct request IDs, ignore obsolete results and cancel every outstanding estimate. All playlist entries load progressively without a hidden count cap. Only the completed `Preview` response confirms the final selectable set. Metadata timeout or cancellation rejects instead of treating a partial directory as complete.

`size` is selected download-stream bytes. `size:null` means unknown, regardless of `approximate`; known `approximate:false` is exact source-reported bytes and `true` is an estimate. If one selected stream is unknown, the total is unknown. Section downloads return unknown estimated transfer size. `disk_estimate` is a rough two-times-transfer working-space allowance, not a measured peak or guaranteed upper bound; transcoding can need more.

Task cancellation never rewrites a completed terminal state. Retry is available for failed/cancelled/interrupted/skipped tasks and uses the original configuration unless `retry_current` is selected. Removal deletes records, not output files. Queue pause persists; tasks left active or queued across process exit recover as `interrupted` and require explicit retry.

## Data shapes

- `Entry`: `{id:string,entry_id?:string,url:string,title:string,index?:number,duration?:number,thumbnail?:string,size:number|null,approximate:boolean,available?:boolean}`. `index` preserves the original collection index. The frontend may make `id` unique per occurrence while preserving the source identifier in `entry_id`.
- `Preview`: `{id,title,url,thumbnail?,duration?,is_playlist:boolean,total?:number,entries:Entry[],formats:Format[],size:number|null,approximate:boolean,format_label?:string,warnings?:string[]}`. Playlist top-level size remains unknown; aggregate selected entries explicitly.
- `Format`: `{id,resolution,height?,fps?,vcodec?,acodec?,size:number|null,approximate:boolean,size_scope:'video_stream'|'combined'}`. A video-only format row excludes its separate audio; use `estimate` for the complete selected download.
- `Task`: `{id,title,url,status,progress?:number|null,downloaded?:number,total?:number|null,speed?:number,eta?:number,index?:number,entry_id?:string,collection_id?:string,batch_id?:string,batch_title?:string,batch_total?:number,batch_count?:number,sequence:number,engine_version?:string,thumbnail?:string,format_label?:string,error?:string|null,output?:string|null,output_media?:object,created_at:number,settings:object,command_args?:string,command_stdin?:string}`. Nullable optional source fields can be JSON null. `created_at` is Unix milliseconds. `sequence` orders the queue. Engine version is recorded from the actual engine when execution begins. Verified output has `output_media.verified:true`; numeric fps, dimensions, codec and output size/duration are provided where available.
- Task statuses: `queued`, `starting`, `downloading`, `processing`, `completed`, `failed`, `cancelled`, `interrupted`, `skipped`.
- `Profile`: `{id,name,settings}`.
- `Engine`: `{version:string,path:string,available:boolean,catalog_version:string,catalog_compatible:boolean,ffmpeg:boolean,ffprobe:boolean,js_runtime:boolean,can_rollback:boolean}`. `catalog_compatible` indicates version equality, not exhaustive compatibility certification.

## Events

- `task`: a complete Task snapshot replacing/inserting one row.
- `tasks`: Task[] replacing the task snapshot.
- `settings`: saved default settings.
- `queue_state`: `{paused:boolean}`.
- `preview_progress`: `{request_id:string,loaded:number,total?:number|null,entries?:Entry[],title?:string|null,is_playlist?:boolean,complete:boolean}`. `entries` is an **append-only new batch**, never the whole replacement list. Fields may be omitted in later batches. The final event has `complete:true`; the successful dispatch response supplies the complete Preview. Do not enable full-collection submission using partial batches.
- `notice`: shell/UI notice `{message:string}`.

Task snapshots persist in SQLite; log text is not sent in task events. Diagnostic logs are masked and bounded. The GUI command tool captures UTF-8 text stdout separately, normalizing line endings; this is not byte-for-byte or binary-stream capture. Stderr is included in masked diagnostic output, not preserved as a separate verbatim stream. Text stdout may contain credentials or signed URLs: its display/export requires the explicit raw-output path in the UI. Use the independent `flow.cmd` CLI for native byte streams, including binary media written with `-o -`; the CLI preserves the engine's stdout/stderr byte behavior and uses its bundled engine independently of the GUI's updated user copy.

## Engine lifecycle and desktop shell

Engine update refuses in-use/queued engines, stages a candidate under the independent user data directory, validates required protocols, then switches it and retains a previous version. Packaged resources are not overwritten. Native `--update`, `-U`, and `--update-to` similarly run against an isolated candidate under an exclusive engine lock. `engine_verify` hashes bundled files against their manifest in a blocking worker; updated user engines are reported separately and are not compared to the bundle hash. Windows Job Objects own running engine trees and terminate descendants on cancellation/exit.

Shell operations: `pick_folder {initial?,title?} -> string|null`; `pick_cookie {initial?,title?} -> string|null`; `pick_file {initial?,title?} -> string|null`; `open_path {path} -> null` (open directory or reveal file in its directory; path must exist and be absolute); `open_licenses {} -> null` (open bundled third-party license folder); `export_file {name,content} -> string|null` via native save dialog; `import_file {} -> {path,content}|null`; `exit` / `request_close` return `{action?:'hide'|'exit',cancelled:boolean}` after close policy and active-task confirmation. Shell operations are handled before delegation to the core. `window_state`, `save_window_state`, and `window_visible` coordinate native window restoration and hidden-state notifications. `bootstrap.shell.tray_available` reflects actual tray availability; a missing tray never causes an unreachable hidden window.

The native UI never substitutes demonstration tasks for a missing core. Testing fixtures use explicit isolated paths. Credentials, logs and configuration are never shared with the legacy web app. A task snapshot includes a monotonic revision so stale asynchronous results cannot overwrite newer task states.

## Native event and isolation details

The service coalesces progress at approximately 100 ms, while terminal, settings and shell events wake immediately. When pending task snapshots overflow, `tasks_invalidated` requests `task_snapshot -> {tasks,revision}`. A preview event with `snapshot_required:true` indicates an omitted intermediate batch; the final successful Preview remains authoritative and uncapped. Neither marker is permission to discard collection entries.

Complete native isolation requires absolute FLOW_ROAMING_DATA_DIR, FLOW_LOCAL_DATA_DIR and FLOW_DOWNLOAD_DIR together; FLOW_DATA_DIR remains a compatible core-only override. In isolated mode OS tray/notification/dialog/Explorer integration is disabled. This mode verifies core and rendering behavior without claiming those OS integrations passed.
