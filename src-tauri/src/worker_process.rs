//! Owned HTTP worker process. Dropping it always closes input and reaps the child.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub(crate) struct Worker {
    child: Child,
    input: Option<ChildStdin>,
    messages: Receiver<Value>,
    pub persistent: bool,
    pub born: Instant,
    pub booking_dispatched: bool,
}

impl Worker {
    pub fn spawn(path: &Path, persistent: bool, readonly: bool) -> Result<Self, String> {
        let mut cmd = Command::new(path);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        if persistent {
            cmd.arg(if readonly {"--serve-readonly"} else {"--serve"});
            cmd.env("XMU_HTTP_PARENT_PID",std::process::id().to_string());
        }
        Self::spawn_command(cmd,persistent)
    }

    fn spawn_command(mut cmd:Command,persistent:bool)->Result<Self,String> {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)] {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let born = Instant::now();
        let mut child = cmd.spawn().map_err(|_| "HTTP_WORKER_START_FAILED".to_string())?;
        let input = child.stdin.take();
        let stdout = match child.stdout.take() {
            Some(value) => value,
            None => {let _=child.kill();let _=child.wait();return Err("HTTP_WORKER_START_FAILED".into());}
        };
        let (sender,messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(body)=line.strip_prefix("@@XMU_HTTP@@") {
                    if let Ok(value)=serde_json::from_str::<Value>(body) {
                        if sender.send(value).is_err() {break;}
                    }
                }
            }
        });
        Ok(Self{child,input,messages,persistent,born,booking_dispatched:false})
    }

    pub fn alive(&mut self) -> bool { matches!(self.child.try_wait(),Ok(None)) }
    pub fn process_id(&self)->u32 {self.child.id()}

    pub fn send(&mut self, payload: &mut Value) -> Result<String,String> {
        if self.booking_dispatched {return Err("SUBMIT_UNKNOWN".into());}
        if !self.alive() {return Err(self.failure_code("HTTP_WARM_WORKER_LOST").into());}
        let rpc_id=uuid::Uuid::new_v4().to_string();
        payload["rpc_id"]=json!(rpc_id);
        // A failed/partial write of a booking request is still ambiguous. Never
        // restart/replay it just because no progress message arrived.
        self.booking_dispatched |= payload["action"]=="book";
        let failure=self.failure_code("HTTP_WORKER_START_FAILED").to_string();
        let result=self.input.as_mut().ok_or("HTTP_WORKER_START_FAILED".to_string())
            .and_then(|input|writeln!(input,"{payload}").map_err(|_|failure));
        if !self.persistent {self.input.take();}
        result.map(|_|rpc_id)
    }

    pub fn receive(&self, duration: Duration) -> Result<Value,RecvTimeoutError> {self.messages.recv_timeout(duration)}
    pub fn failure_code<'a>(&self, before_send:&'a str)->&'a str {if self.booking_dispatched {"SUBMIT_UNKNOWN"} else {before_send}}
}

#[cfg(all(test,windows))]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{CloseHandle,WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{OpenProcess,PROCESS_ACCESS_RIGHTS,WaitForSingleObject};

    fn fixture(script:&str)->Worker {
        let executable=std::path::PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut cmd=Command::new(executable);
        cmd.args(["-NoProfile","-NonInteractive","-Command",script]);
        Worker::spawn_command(cmd,true).unwrap()
    }

    #[test]
    fn persistent_rpc_correlates_messages_and_drop_reaps_an_idle_process() {
        let mut worker=fixture("while($line=[Console]::ReadLine()) {$request=$line|ConvertFrom-Json; [Console]::WriteLine('@@XMU_HTTP@@'+(@{kind='result';rpc_id=$request.rpc_id;ok=$true}|ConvertTo-Json -Compress))}");
        for action in ["warm","ping"] {
            let id=worker.send(&mut json!({"action":action})).unwrap();
            let result=worker.receive(Duration::from_secs(10)).unwrap();
            assert_eq!(result["rpc_id"],id);assert_eq!(result["ok"],true);assert!(worker.alive());
        }
        let handle=unsafe {OpenProcess(PROCESS_ACCESS_RIGHTS(0x00100000),false,worker.child.id()).unwrap()};
        drop(worker);
        unsafe {assert_eq!(WaitForSingleObject(handle,5000),WAIT_OBJECT_0);let _=CloseHandle(handle);}
    }

    #[test]
    fn lost_booking_pipe_stays_unknown_and_never_accepts_a_replay() {
        let mut worker=fixture("$null=[Console]::ReadLine(); exit 7");
        worker.send(&mut json!({"action":"book"})).unwrap();
        assert!(matches!(worker.receive(Duration::from_secs(10)),Err(RecvTimeoutError::Disconnected)));
        assert_eq!(worker.failure_code("HTTP_WORKER_ERROR"),"SUBMIT_UNKNOWN");
        assert_eq!(worker.send(&mut json!({"action":"book"})).unwrap_err(),"SUBMIT_UNKNOWN");
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.input.take();
        let _=self.child.kill();
        let _=self.child.wait();
    }
}
