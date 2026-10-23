use std::sync::atomic::Ordering;
use tauri::Manager;

pub fn run_still_active(app:&tauri::AppHandle,generation:u64)->bool {
    app.try_state::<crate::AppState>().map(|state|state.run_gen.load(Ordering::SeqCst)==generation).unwrap_or(false)
}
pub fn fire_wait_ms(remaining:i64)->u64 {if remaining>1000 {500} else if remaining>100 {50} else {10}}
