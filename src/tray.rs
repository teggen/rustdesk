use crate::client::translate;
#[cfg(windows)]
use crate::ipc::Data;
use hbb_common::tokio;
use hbb_common::{allow_err, log};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
#[cfg(windows)]
use std::time::Duration;

// Absolute hide: admin-deployed `hide-tray` builtin option. The tray process
// does not run at all (except the macOS event loop, kept for parity).
fn tray_hard_hidden() -> bool {
    use hbb_common::config::keys;
    crate::ui_interface::get_builtin_option(keys::OPTION_HIDE_TRAY) == "Y"
}

// Silent direct-IP access hides the tray by default, but a connection may ask to
// override silent mode, in which case the icon is shown while the override is
// active (see `force_show_tray` queried over IPC from the running server).
fn tray_silent_hidden() -> bool {
    use hbb_common::config::keys;
    crate::ui_interface::get_option(keys::OPTION_ALLOW_SILENT_DIRECT_ACCESS) == "Y"
}

// Whether the tray icon should currently be visible.
fn tray_should_show(force_show: bool) -> bool {
    !tray_hard_hidden() && (!tray_silent_hidden() || force_show)
}

// Poll the running server for the live "force show tray" signal. Returns false
// if the server is unreachable (e.g. not running yet).
#[tokio::main(flavor = "current_thread")]
async fn query_force_show_tray() -> bool {
    matches!(
        crate::ipc::get_config("force_show_tray").await,
        Ok(Some(ref v)) if v == "Y"
    )
}

pub fn start_tray() {
    // When silent mode is on we still run the event loop (with a hidden icon) so
    // that an override connection can make the icon appear at runtime. Only the
    // absolute admin "hide-tray" suppresses the tray process entirely.
    if tray_hard_hidden() {
        #[cfg(not(target_os = "macos"))]
        {
            return;
        }
    }

    #[cfg(target_os = "linux")]
    crate::server::check_zombie();

    allow_err!(make_tray());
}

fn make_tray() -> hbb_common::ResultType<()> {
    // https://github.com/tauri-apps/tray-icon/blob/dev/examples/tao.rs
    use hbb_common::anyhow::Context;
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tray_icon::{
        menu::{Menu, MenuEvent, MenuItem},
        TrayIcon, TrayIconBuilder, TrayIconEvent as TrayEvent,
    };
    let icon;
    #[cfg(target_os = "macos")]
    {
        icon = include_bytes!("../res/mac-tray-dark-x2.png"); // use as template, so color is not important
    }
    #[cfg(not(target_os = "macos"))]
    {
        icon = include_bytes!("../res/tray-icon.ico");
    }

    let (icon_rgba, icon_width, icon_height) = {
        let image = load_icon_from_asset()
            .unwrap_or(image::load_from_memory(icon).context("Failed to open icon path")?)
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };
    let icon = tray_icon::Icon::from_rgba(icon_rgba, icon_width, icon_height)
        .context("Failed to open icon")?;

    let mut event_loop = EventLoopBuilder::new().build();

    let tray_menu = Menu::new();
    let hide_stop_service = crate::ui_interface::get_builtin_option(
        hbb_common::config::keys::OPTION_HIDE_STOP_SERVICE,
    ) == "Y";
    // The tray icon is only shown when the service is running, so we don't need to check
    // the `stop-service` option here.
    let quit_i = if !hide_stop_service {
        Some(MenuItem::new(translate("Stop service".to_owned()), true, None))
    } else {
        None
    };
    let open_i = MenuItem::new(translate("Open".to_owned()), true, None);
    if let Some(quit_i) = &quit_i {
        tray_menu.append_items(&[&open_i, quit_i]).ok();
    } else {
        tray_menu.append_items(&[&open_i]).ok();
    }
    let tooltip = |count: usize| {
        if count == 0 {
            format!(
                "{} {}",
                crate::get_app_name(),
                translate("Service is running".to_owned()),
            )
        } else {
            format!(
                "{} - {}\n{}",
                crate::get_app_name(),
                translate("Ready".to_owned()),
                translate("{".to_string() + &format!("{count}") + "} sessions"),
            )
        }
    };
    let mut _tray_icon: Arc<Mutex<Option<TrayIcon>>> = Default::default();

    let menu_channel = MenuEvent::receiver();
    let tray_channel = TrayEvent::receiver();
    #[cfg(windows)]
    let (ipc_sender, ipc_receiver) = std::sync::mpsc::channel::<Data>();

    // Live "force show tray" state, polled from the running server so an override
    // connection can make the icon appear even while silent mode is enabled.
    let force_show = Arc::new(AtomicBool::new(false));
    {
        let force_show = force_show.clone();
        std::thread::spawn(move || loop {
            let v = query_force_show_tray();
            force_show.store(v, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_secs(1));
        });
    }
    // Track the last applied visibility to avoid redundant updates.
    let mut last_show = !tray_silent_hidden();

    let open_func = move || {
        if cfg!(not(feature = "flutter")) {
            crate::run_me::<&str>(vec![]).ok();
            return;
        }
        #[cfg(target_os = "macos")]
        crate::platform::macos::handle_application_should_open_untitled_file();
        #[cfg(target_os = "windows")]
        {
            // Do not use "start uni link" way, it may not work on some Windows, and pop out error
            // dialog, I found on one user's desktop, but no idea why, Windows is shit.
            // Use `run_me` instead.
            // `allow_multiple_instances` in `flutter/windows/runner/main.cpp` allows only one instance without args.
            crate::run_me::<&str>(vec![]).ok();
        }
        #[cfg(target_os = "linux")]
        {
            // Do not use "xdg-open", it won't read the config.
            if crate::dbus::invoke_new_connection(crate::get_uri_prefix()).is_err() {
                if let Ok(task) = crate::run_me::<&str>(vec![]) {
                    crate::server::CHILD_PROCESS.lock().unwrap().push(task);
                }
            }
        }
    };

    #[cfg(windows)]
    std::thread::spawn(move || {
        start_query_session_count(ipc_sender.clone());
    });
    #[cfg(windows)]
    let mut last_click = std::time::Instant::now();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::EventLoopExtMacOS;
        event_loop.set_activation_policy(tao::platform::macos::ActivationPolicy::Accessory);
    }
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(100),
        );

        if let tao::event::Event::NewEvents(tao::event::StartCause::Init) = event {
            // for fixing https://github.com/rustdesk/rustdesk/discussions/10210#discussioncomment-14600745
            // so we start tray, but not to show it
            if tray_hard_hidden() {
                return;
            }
            // We create the icon once the event loop is actually running
            // to prevent issues like https://github.com/tauri-apps/tray-icon/issues/90
            let show = tray_should_show(force_show.load(Ordering::SeqCst));
            last_show = show;
            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(tray_menu.clone()))
                .with_tooltip(tooltip(0))
                .with_icon(icon.clone())
                .with_icon_as_template(true) // mac only
                .build();
            match tray {
                Ok(tray) => {
                    // Apply the initial visibility (hidden while silent mode is on
                    // and no override session is active).
                    allow_err!(tray.set_visible(show));
                    _tray_icon = Arc::new(Mutex::new(Some(tray)))
                }
                Err(err) => {
                    log::error!("Failed to create tray icon: {}", err);
                }
            };

            // We have to request a redraw here to have the icon actually show up.
            // Tao only exposes a redraw method on the Window so we use core-foundation directly.
            #[cfg(target_os = "macos")]
            unsafe {
                use core_foundation::runloop::{CFRunLoopGetMain, CFRunLoopWakeUp};

                let rl = CFRunLoopGetMain();
                CFRunLoopWakeUp(rl);
            }
        }

        // Re-evaluate tray visibility (silent mode vs. an active override session).
        let show = tray_should_show(force_show.load(Ordering::SeqCst));
        if show != last_show {
            last_show = show;
            if let Some(t) = _tray_icon.lock().unwrap().as_ref() {
                allow_err!(t.set_visible(show));
            }
        }

        if let Ok(event) = menu_channel.try_recv() {
            if let Some(quit_i) = &quit_i {
                if event.id == quit_i.id() {
                    /* failed in windows, seems no permission to check system process
                    if !crate::check_process("--server", false) {
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                    */
                    if !crate::platform::uninstall_service(false, false) {
                        *control_flow = ControlFlow::Exit;
                    }
                } else if event.id == open_i.id() {
                    open_func();
                }
            } else if event.id == open_i.id() {
                open_func();
            }
        }

        if let Ok(_event) = tray_channel.try_recv() {
            #[cfg(target_os = "windows")]
            match _event {
                TrayEvent::Click {
                    button,
                    button_state,
                    ..
                } => {
                    if button == tray_icon::MouseButton::Left
                        && button_state == tray_icon::MouseButtonState::Up
                    {
                        if last_click.elapsed() < std::time::Duration::from_secs(1) {
                            return;
                        }
                        open_func();
                        last_click = std::time::Instant::now();
                    }
                }
                _ => {}
            }
        }

        #[cfg(windows)]
        if let Ok(data) = ipc_receiver.try_recv() {
            match data {
                Data::ControlledSessionCount(count) => {
                    _tray_icon
                        .lock()
                        .unwrap()
                        .as_mut()
                        .map(|t| t.set_tooltip(Some(tooltip(count))));
                }
                _ => {}
            }
        }
    });
}

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn start_query_session_count(sender: std::sync::mpsc::Sender<Data>) {
    let mut last_count = 0;
    loop {
        if let Ok(mut c) = crate::ipc::connect(1000, "").await {
            let mut timer = crate::rustdesk_interval(tokio::time::interval(Duration::from_secs(1)));
            loop {
                tokio::select! {
                    res = c.next() => {
                        match res {
                            Err(err) => {
                                log::error!("ipc connection closed: {}", err);
                                break;
                            }

                            Ok(Some(Data::ControlledSessionCount(count))) => {
                                if count != last_count {
                                    last_count = count;
                                    sender.send(Data::ControlledSessionCount(count)).ok();
                                }
                            }
                            _ => {}
                        }
                    }

                    _ = timer.tick() => {
                        c.send(&Data::ControlledSessionCount(0)).await.ok();
                    }
                }
            }
        }
        hbb_common::sleep(1.).await;
    }
}

fn load_icon_from_asset() -> Option<image::DynamicImage> {
    let Some(path) = std::env::current_exe().map_or(None, |x| x.parent().map(|x| x.to_path_buf()))
    else {
        return None;
    };
    #[cfg(target_os = "macos")]
    let path = path.join("../Frameworks/App.framework/Resources/flutter_assets/assets/icon.png");
    #[cfg(windows)]
    let path = path.join(r"data\flutter_assets\assets\icon.png");
    #[cfg(target_os = "linux")]
    let path = path.join(r"data/flutter_assets/assets/icon.png");
    if path.exists() {
        if let Ok(image) = image::open(path) {
            return Some(image);
        }
    }
    None
}
