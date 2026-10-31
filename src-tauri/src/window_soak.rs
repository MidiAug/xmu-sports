//! Explicit, offline Windows/WebView2 soak. No booking or login commands registered.
use super::*;
use std::io::Write;
use std::sync::{mpsc, atomic::AtomicUsize};

struct Samples {
    count: AtomicUsize,
    errors: Mutex<Vec<String>>,
}

#[tauri::command]
fn soak_ack(state: State<'_, Samples>, errors: Vec<String>) {
    state.errors.lock().unwrap().extend(errors);
    state.count.fetch_add(1, Ordering::SeqCst);
}

fn record(path: &std::path::Path, value: Value) {
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(file, "{value}").unwrap();
    println!("[window_soak] {value}");
}

fn foreground(window: &tauri::WebviewWindow) -> bool {
    // A WebView child may own keyboard focus while its top-level window is
    // still foreground. Tauri is_focused() alone does not establish that state.
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() == window.hwnd().unwrap() }
}

#[test]
#[ignore = "interactive isolated WebView2 soak; requires XMU_SOAK_URL and a fresh XMU_SPORTS_DATA"]
fn native_window_soak() {
    let url = std::env::var("XMU_SOAK_URL").expect("isolated local Vite URL required");
    assert!(url.starts_with("http://127.0.0.1:"));
    let root = PathBuf::from(std::env::var("XMU_SPORTS_DATA").expect("fresh isolated data directory required"));
    assert!(!root.exists(), "use a new data directory, never an existing profile");
    fs::create_dir_all(&root).unwrap();
    let report = root.join("samples.jsonl");
    let phase_secs: u64 = std::env::var("XMU_SOAK_PHASE_SECONDS").unwrap_or_else(|_| "300".into()).parse().unwrap();
    assert!(phase_secs > 0);
    let phases = std::env::var("XMU_SOAK_PHASES").unwrap_or_else(|_| "foreground,background,minimized,hidden".into());
    assert!(phases.split(',').all(|phase| matches!(phase, "foreground" | "background" | "minimized" | "hidden" | "idle")));
    assert!(!phases.split(',').any(|phase| phase == "idle") || phases == "idle", "idle must run alone without pressure workers");
    if phases == "idle" {
        let catalog: Value = serde_json::from_str(include_str!("../../src/catalog/venues.json")).unwrap();
        let sport = catalog["sports"].as_array().unwrap().iter().find(|s| s["enabled"] != false).unwrap();
        let venue = &sport["venues"][0];
        let slot = &venue["time_slots"][0];
        // Keep the real enabled-plan health loop, with a synthetic account and
        // a first date far outside this test. It cannot become due during idle.
        fs::write(root.join("booking-plans.json"), serde_json::to_vec(&serde_json::json!({
            "rules":[{"id":uuid::Uuid::new_v4().to_string(),"sport":sport["id"],"venue":venue["id"],
                "start":slot["start"],"end":slot["end"],"weekdays":[1,2,3,4,5,6,7],"enabled":true,
                "account":"a".repeat(64),"first_date":(booking_plans::china_now().date_naive()+chrono::Duration::days(30)).to_string()}]
        })).unwrap()).unwrap();
    }
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().build.dev_url = Some(url.parse().unwrap());
    let result = Arc::new(Mutex::new(false));
    let completed = result.clone();
    tauri::Builder::default().any_thread()
        .manage(AppState { arm_gen: AtomicU64::new(0), run_gen: AtomicU64::new(0),
            native_running: AtomicBool::new(false), stopping: AtomicBool::new(false),
            task_gate: Mutex::new(()), active_task: Mutex::new(None), active_operation: Mutex::new(None) })
        .manage(Samples { count: AtomicUsize::new(0), errors: Mutex::new(Vec::new()) })
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Warn)
            .targets([tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout)]).build())
        .invoke_handler(tauri::generate_handler![soak_ack, get_live_status, get_shell_prefs,
            read_task_raw, probe_wechat, set_run_badge, list_pending_orders, list_recent_events,
            booking_plans::get_booking_plans])
        .setup(move |app| {
            tray::setup_tray(app.handle())?;
            booking_plans::start(app.handle());
            let main = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url.parse()?))
                .title("XMU Sports [isolated soak]").inner_size(1100.0, 760.0)
                .data_directory(root.join("webview"))
                .initialization_script("localStorage.setItem('xmu.booking.mode','plans');")
                .build()?;
            let cover = tauri::WebviewWindowBuilder::new(app, "soak-cover", tauri::WebviewUrl::External("about:blank".parse()?))
                .title("XMU Sports [background test cover]").inner_size(1140.0, 800.0).visible(false)
                .data_directory(root.join("cover-webview")).build()?;
            let handle = app.handle().clone();
            thread::spawn(move || {
                let abort = |message: &str| -> ! {
                    record(&report, serde_json::json!({"failure":message}));
                    // A deadlocked event loop cannot handle app.exit; this is a
                    // dedicated ignored test process, never the user's app.
                    std::process::exit(2);
                };
                let stop = Arc::new(AtomicBool::new(false));
                let cycles = Arc::new(AtomicUsize::new(0));
                let stress_app = handle.clone();
                let stress_stop = stop.clone();
                let stress_cycles = cycles.clone();
                let stress = (phases != "idle").then(|| thread::spawn(move || {
                    while !stress_stop.load(Ordering::SeqCst) {
                        let runtime = stress_app.state::<AppState>();
                        let _gate = runtime.task_gate.lock().unwrap();
                        // Same native diagnostics and lock context as 06:59 preparation.
                        let _ = wechat_window::device_diagnostics();
                        sync_run_badge(&stress_app, &runtime);
                        tray::refresh_plans(&stress_app);
                        let cycle = stress_cycles.fetch_add(1, Ordering::SeqCst);
                        if cycle % 100 == 0 {
                            let trace = run_trace::RunTraceHandle::open(&runs_dir(), "offline-soak", &format!("cycle-{cycle}"),
                                serde_json::json!({"offline":true}), Some("diagnostics-only"), Default::default()).unwrap();
                            trace.finish("succeeded", Some("offline-diagnostics"));
                        }
                        drop(_gate);
                        thread::sleep(Duration::from_millis(100));
                    }
                }));
                // Let the real application module load before the first UI sample.
                thread::sleep(Duration::from_secs(3));
                for phase in phases.split(',') {
                    cover.hide().unwrap();
                    main.show().unwrap();
                    main.unminimize().unwrap();
                    match phase {
                        "foreground" => main.set_focus().unwrap(),
                        "background" => { cover.show().unwrap(); cover.set_focus().unwrap(); },
                        "minimized" => main.minimize().unwrap(),
                        "hidden" | "idle" => main.hide().unwrap(),
                        _ => unreachable!(),
                    }
                    if phase == "foreground" && phase_secs >= 60 {
                        record(&report, serde_json::json!({"phase":phase,"waiting_for_focus":true}));
                        let waiting = Instant::now();
                        while !foreground(&main) {
                            if waiting.elapsed() > Duration::from_secs(60) { abort("foreground activation required before timing"); }
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                    record(&report, serde_json::json!({"phase":phase,"started":true,"time":Local::now().to_rfc3339()}));
                    let start = Instant::now();
                    let initial = handle.state::<Samples>().count.load(Ordering::SeqCst);
                    let mut polls = 0;
                    let mut max_native_ms = 0;
                    let mut max_webview_ms = 0;
                    let mut states = std::collections::BTreeMap::new();
                    while start.elapsed() < Duration::from_secs(phase_secs) {
                        if phase == "idle" {
                            // No injected JS, native heartbeat or extra UI work:
                            // only the application's own timers/scheduler remain.
                            thread::sleep(Duration::from_secs(1));
                            if start.elapsed().as_secs() % 30 == 0 {
                                record(&report, serde_json::json!({"phase":phase,"elapsed_s":start.elapsed().as_secs(),"injected_polls":0}));
                            }
                            continue;
                        }
                        let (tx, rx) = mpsc::channel();
                        let app = handle.clone();
                        let t = Instant::now();
                        handle.run_on_main_thread(move || {
                            // Real synchronous command on the actual native UI thread.
                            let status = get_live_status(app.state::<AppState>());
                            let _ = tx.send(status.is_ok());
                        }).unwrap();
                        if rx.recv_timeout(Duration::from_secs(5)) != Ok(true) { abort("native task-state poll stalled"); }
                        max_native_ms = max_native_ms.max(t.elapsed().as_millis());
                        let before = handle.state::<Samples>().count.load(Ordering::SeqCst);
                        let script = r#"(async()=>{const errors=[];try{
                            if(!document.querySelector('.app-header'))throw Error('application not rendered');
                            const invoke=window.__TAURI_INTERNALS__.invoke;
                            await Promise.all(['get_live_status','get_booking_plans','probe_wechat'].map(c=>invoke(c)));
                            const tabs=['plans','live','history','settings'];
                            const tab=tabs[Number(document.documentElement.dataset.soakTick||0)%tabs.length];
                            const button=document.querySelector(`[data-tab="${tab}"]`);
                            if(!button)throw Error('missing navigation '+tab);
                            button.click();
                            document.documentElement.dataset.soakTick=String(Number(document.documentElement.dataset.soakTick||0)+1);
                            await new Promise(resolve=>setTimeout(resolve,50));
                            if(!document.querySelector('.content')?.textContent.trim())throw Error('empty page after navigation');
                            const expected={plans:'长期计划',live:'预约进度',history:'活动记录',settings:'设置'}[tab];
                            if(document.querySelector('#panel .page-heading h2')?.textContent!==expected)throw Error('navigation did not render '+tab);
                        }catch(e){errors.push(String(e))}await window.__TAURI_INTERNALS__.invoke('soak_ack',{errors});})()"#;
                        let t = Instant::now();
                        main.eval(script).unwrap();
                        while handle.state::<Samples>().count.load(Ordering::SeqCst) == before {
                            if t.elapsed() > Duration::from_secs(5) { abort("WebView/IPC/navigation stalled"); }
                            thread::sleep(Duration::from_millis(10));
                        }
                        max_webview_ms = max_webview_ms.max(t.elapsed().as_millis());
                        let errors = handle.state::<Samples>().errors.lock().unwrap().clone();
                        if !errors.is_empty() { abort(&format!("WebView errors: {errors:?}")); }
                        let state = format!("visible={} minimized={} focused={} foreground={}", main.is_visible().unwrap(), main.is_minimized().unwrap(), main.is_focused().unwrap(), foreground(&main));
                        *states.entry(state).or_insert(0usize) += 1;
                        polls += 1;
                        if polls % 30 == 0 {
                            record(&report, serde_json::json!({"phase":phase,"elapsed_s":start.elapsed().as_secs(),"polls":polls,"stress_cycles":cycles.load(Ordering::SeqCst)}));
                        }
                        thread::sleep(Duration::from_millis(200));
                    }
                    record(&report, serde_json::json!({"phase":phase,"complete":true,"elapsed_s":start.elapsed().as_secs(),
                        "native_polls":polls,"webview_acks":handle.state::<Samples>().count.load(Ordering::SeqCst)-initial,
                        "max_native_ms":(polls>0).then_some(max_native_ms),"max_webview_ms":(polls>0).then_some(max_webview_ms),"observed_states":states,
                        "stress_cycles":cycles.load(Ordering::SeqCst)}));
                    if matches!(phase, "minimized" | "hidden" | "idle") {
                        let before = handle.state::<Samples>().count.load(Ordering::SeqCst);
                        let restoring = Instant::now();
                        // Exercise the same implementation as reopening from tray.
                        tray::show_main(&handle);
                        main.eval(r#"(async()=>{const errors=[];try{
                            await window.__TAURI_INTERNALS__.invoke('get_live_status');
                            document.querySelector('[data-tab="plans"]').click();
                            await new Promise(resolve=>setTimeout(resolve,50));
                            if(document.querySelector('#panel .page-heading h2')?.textContent!=='长期计划')throw Error('restore navigation failed');
                        }catch(e){errors.push(String(e))}await window.__TAURI_INTERNALS__.invoke('soak_ack',{errors});})()"#).unwrap();
                        while handle.state::<Samples>().count.load(Ordering::SeqCst) == before {
                            if restoring.elapsed() > Duration::from_secs(5) { abort("restore from tray/minimize stalled"); }
                            thread::sleep(Duration::from_millis(10));
                        }
                        if !handle.state::<Samples>().errors.lock().unwrap().is_empty()
                            || !main.is_visible().unwrap() || main.is_minimized().unwrap() {
                            abort("restored window was not usable");
                        }
                        record(&report, serde_json::json!({"phase":phase,"restored":true,"restore_ms":restoring.elapsed().as_millis()}));
                    }
                }
                stop.store(true, Ordering::SeqCst);
                if let Some(stress) = stress { stress.join().unwrap(); }
                *completed.lock().unwrap() = true;
                handle.exit(0);
            });
            Ok(())
        }).build(context).unwrap().run_return(|_, _| {});
    assert!(*result.lock().unwrap(), "test window closed before soak completed");
}
