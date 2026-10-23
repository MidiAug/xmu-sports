//! Local pending-record inventory. Recheck itself always re-loads in the worker.
use serde_json::{json,Value};
use std::path::Path;

pub(crate) const TRACE_TASK:&str="order-rechecks";

pub(crate) fn valid_reference(reference:&str)->bool {
    reference.len()==64 && reference.bytes().all(|byte|byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn list(root:&Path)->Vec<Value> {
    let directory=root.join("http-booking/requests");
    let mut files:Vec<_>=std::fs::read_dir(&directory).into_iter().flatten().flatten()
        .filter(|entry|entry.path().extension().and_then(|value|value.to_str())==Some("json"))
        .filter_map(|entry|entry.metadata().ok().map(|meta|(meta.modified().ok(),entry.path(),meta.len())))
        .collect();
    files.sort_by(|a,b|b.0.cmp(&a.0));
    let mut output=Vec::new();
    for (_,path,length) in files {
        if output.len()>=50 {break;}
        let Some(reference)=path.file_stem().and_then(|value|value.to_str()).filter(|value|valid_reference(value)) else {continue;};
        let record=if length<=1024*1024 {std::fs::read(&path).ok().and_then(|raw|serde_json::from_slice::<Value>(&raw).ok())} else {None};
        let Some(record)=record.filter(Value::is_object) else {
            output.push(json!({"request_ref":reference,"can_recheck":false,"reason":"记录损坏，请在小程序核对"}));
            continue;
        };
        if !matches!(record["state"].as_str(),Some("started"|"unknown"|"submitted")) || record["verified"]==true {continue;}
        let actual=&record["submitted_target"];
        let complete=["sport","venue","date","time","end","court"].iter()
            .all(|key|actual[*key].as_str().is_some_and(|value|!value.trim().is_empty()));
        let bound=record["account_fingerprint"].as_str().is_some_and(valid_reference);
        output.push(json!({"request_ref":reference,"target":if actual.is_object() {actual} else {&record["target"]},
            "can_recheck":complete && bound,"reason":if complete && bound {""} else {"旧记录缺少原账号或实际场次快照，请在小程序核对"},
            "last_checked_at":record["last_recheck_at"]}));
        if output.len()>=50 {break;}
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_preserves_pending_and_refuses_to_guess_auto_court() {
        let root=std::env::temp_dir().join(format!("xmu-pending-{}",uuid::Uuid::new_v4()));
        let directory=root.join("http-booking/requests");std::fs::create_dir_all(&directory).unwrap();
        let reference="a".repeat(64);
        let record=json!({"state":"unknown","account_fingerprint":"b".repeat(64),
            "target":{"court":null},"submitted_target":{"sport":"运动","venue":"原场馆","date":"2026-10-21","time":"10:00","end":"11:00","court":"04"}});
        let path=directory.join(format!("{reference}.json"));
        std::fs::write(&path,record.to_string()).unwrap();
        let rows=list(&root);assert_eq!(rows.len(),1);assert_eq!(rows[0]["target"]["court"],"04");assert_eq!(rows[0]["can_recheck"],true);
        let mut legacy=record.clone();legacy.as_object_mut().unwrap().remove("submitted_target");
        std::fs::write(&path,legacy.to_string()).unwrap();assert_eq!(list(&root)[0]["can_recheck"],false);
        let mut done=record;done["verified"]=json!(true);std::fs::write(&path,done.to_string()).unwrap();assert!(list(&root).is_empty());
        assert!(!valid_reference("../elsewhere"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
