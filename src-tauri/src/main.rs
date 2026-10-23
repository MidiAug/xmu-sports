#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args().skip(1);
    if let Some(mode) = args.next() {
        if mode=="--login-minimized-check-only" {
            let Some(output)=args.next().map(std::path::PathBuf::from) else {std::process::exit(1);};
            if xmu_sports_lib::login_minimized_check_only(&output).is_err() {std::process::exit(1);}
            return;
        }
        if mode=="--warm-check-only" || mode=="--order-recheck-only" {
            let reference=if mode=="--order-recheck-only" {args.next()} else {None};
            if mode=="--order-recheck-only" && reference.is_none() {std::process::exit(1);}
            let Some(output)=args.next().map(std::path::PathBuf::from) else {std::process::exit(1);};
            if xmu_sports_lib::worker_check_only(reference.as_deref(),&output).is_err() {std::process::exit(1);}
            return;
        }
        if mode == "--login-check-only" || mode == "--login-ui-check-only" || mode == "--login-elements-only" {
            // Deliberately return before app setup/tray/scheduler initialization.
            let output = args.next().map(std::path::PathBuf::from).unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into()))
                    .join("xmu-sports/login-check-only.json")
            });
            let result=if mode=="--login-elements-only" { xmu_sports_lib::login_elements_only(&output) }
                else { xmu_sports_lib::login_check_only(mode == "--login-ui-check-only", &output) };
            if result.is_err() {
                std::process::exit(1);
            }
            return;
        }
    }
    xmu_sports_lib::run()
}
