// SPDX-License-Identifier: Apache-2.0
use flow_platform::lifecycle;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = (|| -> Result<i32, String> {
        #[cfg(target_os = "macos")]
        if args
            .first()
            .is_some_and(|arg| arg == flow_platform::mac_maintenance::HELPER_FLAG)
        {
            if args.len() != 7 {
                return Err("清理工具只能从流光设置中启动".into());
            }
            let parent = args[1]
                .to_str()
                .ok_or("清理进程参数无效")?
                .parse::<u32>()
                .map_err(|_| "清理进程参数无效")?;
            let nonce = args[2].to_str().ok_or("清理授权参数无效")?;
            let paths = lifecycle::Paths::for_locations(
                (&args[3]).into(),
                (&args[4]).into(),
                (&args[5]).into(),
                (&args[6]).into(),
            )?;
            flow_platform::mac_maintenance::run_cleanup_helper(&paths, parent, nonce)?;
            return Ok(0);
        }
        #[cfg(not(windows))]
        if args.first().is_some_and(|arg| {
            [
                "--flow-install-payload",
                "--flow-check-processes",
                "--flow-check-shortcut",
                "--flow-register-shortcut",
                "--flow-check-install-root",
                "--flow-uninstall-cleanup",
                "--flow-clean-legacy-ui-cache",
            ]
            .iter()
            .any(|flag| arg == *flag)
        }) {
            return Err("此内部操作仅供 Windows 安装器使用".into());
        }
        if args
            .first()
            .is_some_and(|arg| arg == "--flow-print-resource-manifest")
        {
            if args.len() != 1 {
                return Err("未知清单参数".into());
            }
            print!("{}", lifecycle::NATIVE_RESOURCE_MANIFEST);
            return Ok(0);
        }
        if args
            .first()
            .is_some_and(|arg| arg == "--flow-install-payload")
        {
            if args.len() != 4 {
                return Err("缺少安装目录、暂存目录或载荷清单".into());
            }
            flow_platform::installer_payload::install(
                std::path::Path::new(&args[1]),
                std::path::Path::new(&args[2]),
                std::path::Path::new(&args[3]),
            )?;
            return Ok(0);
        }
        if args
            .first()
            .is_some_and(|arg| arg == "--flow-check-processes")
        {
            if args.len() != 2 {
                return Err("缺少安装目录".into());
            }
            flow_platform::installer::check_processes(std::path::Path::new(&args[1]))?;
            return Ok(0);
        }
        if args
            .first()
            .is_some_and(|arg| arg == "--flow-check-shortcut" || arg == "--flow-register-shortcut")
        {
            if args.len() != 3 {
                return Err("缺少快捷方式或目标路径".into());
            }
            let link = std::path::Path::new(&args[1]);
            let target = std::path::Path::new(&args[2]);
            if args[0] == "--flow-register-shortcut" {
                flow_platform::installer::register_shortcut(link, target)?;
            } else {
                flow_platform::installer::check_shortcut(link, target)?;
            }
            return Ok(0);
        }
        if args
            .first()
            .is_some_and(|arg| arg == "--flow-check-install-root")
        {
            if args.len() != 2 {
                return Err("缺少安装目录".into());
            }
            lifecycle::check_install_root(std::path::Path::new(&args[1]))?;
            return Ok(0);
        }
        let paths = lifecycle::Paths::system()?;
        #[cfg(target_os = "macos")]
        let _maintenance = flow_platform::mac_maintenance::acquire_shared(&paths)?;
        if args
            .first()
            .is_some_and(|arg| arg == "--flow-clean-legacy-ui-cache")
        {
            if args.len() != 1 {
                return Err("未知旧界面缓存清理参数".into());
            }
            let removed = lifecycle::cleanup_obsolete_webview(&paths)?;
            println!(
                "{}",
                serde_json::json!({"ok":true,"obsolete_webview_removed":removed})
            );
            return Ok(0);
        }
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let install = executable.parent().ok_or("无法取得引擎位置")?;
        let engine = lifecycle::engine_directory(&executable)?;
        if args
            .first()
            .is_some_and(|arg| arg == "--flow-initialize-data")
        {
            if args.len() != 1 {
                return Err("未知初始化参数".into());
            }
            #[cfg(windows)]
            lifecycle::record_installation(install)?;
            paths.initialize()?;
            let core = flow_core::Core::new_with_cache_root(
                paths.data_base.clone(),
                engine,
                paths.downloads.clone(),
                vec![paths.local.clone(), paths.roaming.clone()],
                paths.core_cache_root(),
                std::sync::Arc::new(|_| {}),
            )?;
            paths.register_data()?;
            core.shutdown();
            return Ok(0);
        }
        if args
            .first()
            .is_some_and(|arg| arg == "--flow-uninstall-cleanup")
        {
            #[cfg(windows)]
            lifecycle::check_install_root(install)?;
            let keep = args.iter().skip(1).any(|arg| arg == "--keep-data");
            if args.iter().skip(1).any(|arg| arg != "--keep-data") {
                return Err("未知清理参数".into());
            }
            let mut report = lifecycle::cleanup(&paths, keep)?;
            report["notification_history_cleared"] =
                serde_json::json!(lifecycle::clear_notification_history().is_ok());
            println!("{report}");
            return Ok(0);
        }
        lifecycle::run_cli(&paths, &engine, args)
    })();
    match result {
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("流光：{message}");
            std::process::exit(1);
        }
    }
}
