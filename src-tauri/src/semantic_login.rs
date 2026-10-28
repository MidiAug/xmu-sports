//! Bounded Windows UI Automation for XMU login controls, without CDP or OCR.
//! Never serialize provider values: document URLs can contain login tickets.
use serde::Serialize;
use crate::wechat_window::NativeSurface;
use windows::core::Interface;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::Foundation::{POINT,WPARAM,LPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, ChildWindowFromPointEx, GetWindowThreadProcessId, GetForegroundWindow, CWP_SKIPINVISIBLE, CWP_SKIPDISABLED, PostMessageW, WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, GA_ROOT};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext,DPI_AWARENESS_CONTEXT,DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

const APP_ORIGIN: &str = "https://servicewechat.com/wx81a2b2fa90759cb7/";
const H5_PREFIX: &str = "https://xdty.xmu.edu.cn/bdlp_h5_fitness_test/view/stadium/";
const MAX_NODES: i32 = 768;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Control { Login, Agreement, Authorize, Stadium }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "action", content = "control", rename_all = "snake_case")]
pub(crate) enum Decision { Handoff, Activate(Control), Wait, Unsupported }

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize)]
#[serde(rename_all="snake_case")]
pub(crate) enum SearchPage { Login, Authorization, Home }
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize)]
#[serde(rename_all="snake_case")]
pub(crate) enum ScrollDirection { Down, Up }

#[derive(Debug, Serialize)]
pub(crate) struct ControlEvidence {
    control: Control, enabled: bool, selected: Option<bool>,
    invoke: bool, toggle: bool, select: bool, legacy_action: bool,
    element_bounds_available: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct Observation {
    pub decision: Decision,
    pub target_app_verified: bool,
    pub home_page: bool,
    pub search_page: Option<SearchPage>,
    pub next_control: Option<Control>,
    pub at_top: Option<bool>,
    pub at_bottom: Option<bool>,
    #[serde(skip)]
    pub viewport_mark: Option<u64>,
    nodes_examined: i32,
    truncated: bool,
    controls: Vec<ControlEvidence>,
}

impl Observation { pub(crate) fn has_controls(&self)->bool {!self.controls.is_empty()} }

#[cfg(test)]
impl Observation {
    pub(crate) fn fixture(h5:bool,controls:&[(Control,bool,Option<bool>)])->Self {
        Self{decision:choose(h5,controls),target_app_verified:true,home_page:false,
            search_page:if h5 {None} else if controls.iter().any(|(c,_,_)|matches!(c,Control::Authorize|Control::Agreement)) {Some(SearchPage::Authorization)}
                else if controls.iter().any(|(c,_,_)|*c==Control::Login) {Some(SearchPage::Login)}
                else if controls.iter().any(|(c,_,_)|*c==Control::Stadium) {Some(SearchPage::Home)} else {None},
            next_control:match choose(h5,controls) {Decision::Activate(c)=>Some(c),_=>None},
            at_top:None,at_bottom:None,viewport_mark:None,nodes_examined:77,truncated:false,
            controls:controls.iter().map(|(control,enabled,selected)|ControlEvidence{control:*control,
                enabled:*enabled,selected:*selected,invoke:true,toggle:false,select:false,legacy_action:false,
                element_bounds_available:true}).collect()}
    }
}

struct Candidate { control: Control, element: IUIAutomationElement, evidence: ControlEvidence }
struct Scan { report: Observation, candidates: Vec<Candidate>, document: Option<IUIAutomationElement>, scroll_container: Option<IUIAutomationScrollPattern> }

struct DpiGuard(DPI_AWARENESS_CONTEXT);
impl DpiGuard {
    fn enter()->Self { Self(unsafe{SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)}) }
}
impl Drop for DpiGuard {
    fn drop(&mut self) { if !self.0.0.is_null() { unsafe{let _=SetThreadDpiAwarenessContext(self.0);} } }
}

fn choose(h5: bool, controls: &[(Control, bool, Option<bool>)]) -> Decision {
    if h5 { return Decision::Handoff; }
    let has = |control| controls.iter().any(|(c, _, _)| *c == control);
    let enabled = |control| controls.iter().any(|(c, enabled, _)| *c == control && *enabled);
    if has(Control::Authorize) || has(Control::Agreement) {
        let agreements: Vec<_> = controls.iter().filter(|(c,_,_)| *c == Control::Agreement).collect();
        // An actual radio/checkbox state is required; never blindly toggle.
        if agreements.len() == 1 && agreements[0].2 == Some(false) && agreements[0].1 {
            return Decision::Activate(Control::Agreement);
        }
        if enabled(Control::Authorize) && agreements.len() == 1 && agreements[0].2 == Some(true) {
            return Decision::Activate(Control::Authorize);
        }
        return Decision::Unsupported;
    }
    if enabled(Control::Login) { return Decision::Activate(Control::Login); }
    if enabled(Control::Stadium) { return Decision::Activate(Control::Stadium); }
    Decision::Wait
}

pub(crate) struct Session { api: IUIAutomation, surface: NativeSurface }
impl Session {
    pub(crate) fn open() -> Result<Self, String> {
        let surfaces = crate::wechat_window::discover_wechat_candidates().map_err(|_| "UIA_DISCOVERY_FAILED")?;
        let mut matching=surfaces.into_iter().filter(|s| s.visible && s.title.contains("厦大体育"));
        let surface=matching.next().ok_or("UIA_TARGET_MISSING")?;
        if matching.next().is_some() { return Err("UIA_TARGET_AMBIGUOUS".into()); }
        unsafe {
            // Already initialized WinRT/COM threads are also valid callers.
            let _=CoInitializeEx(None,COINIT_MULTITHREADED);
            let api: IUIAutomation=CoCreateInstance(&CUIAutomation8,None,CLSCTX_INPROC_SERVER)
                .map_err(|_|"UIA_UNAVAILABLE")?;
            if let Ok(timed)=api.cast::<IUIAutomation2>() {
                let _=timed.SetConnectionTimeout(1000);
                let _=timed.SetTransactionTimeout(2000);
            }
            Ok(Self{api,surface})
        }
    }

    fn validate_window(&self) -> Result<(),String> {
        let surfaces=crate::wechat_window::discover_wechat_candidates().map_err(|_|"UIA_DISCOVERY_FAILED")?;
        if surfaces.iter().any(|s| s.visible && s.hwnd_u64==self.surface.hwnd_u64 && s.owner==self.surface.owner && s.title.contains("厦大体育")) {
            Ok(())
        } else { Err("UIA_WINDOW_CHANGED".into()) }
    }

    unsafe fn value(element: &IUIAutomationElement) -> String {
        if let Ok(value)=element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            if let Ok(value)=value.CurrentValue() { return value.to_string(); }
        }
        element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
            .and_then(|p|p.CurrentValue()).map(|v|v.to_string()).unwrap_or_default()
    }

    unsafe fn visible(element: &IUIAutomationElement) -> bool {
        if element.CurrentIsOffscreen().map(|v|v.as_bool()).unwrap_or(true) { return false; }
        element.CurrentBoundingRectangle().map(|r|r.right>r.left && r.bottom>r.top).unwrap_or(false)
    }

    unsafe fn in_app(&self, element: &IUIAutomationElement) -> bool {
        let Ok(walker)=self.api.RawViewWalker() else { return false; };
        let mut current=element.clone();
        for _ in 0..32 {
            if current.CurrentControlType().ok()==Some(UIA_DocumentControlTypeId) &&
                Self::value(&current).starts_with(APP_ORIGIN) { return true; }
            match walker.GetParentElement(&current) { Ok(parent)=>current=parent, Err(_)=>break }
        }
        false
    }

    unsafe fn selected(element: &IUIAutomationElement) -> Option<bool> {
        if let Ok(pattern)=element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
            if let Ok(state)=pattern.CurrentToggleState() {
                if state==ToggleState_On { return Some(true); }
                if state==ToggleState_Off { return Some(false); }
            }
        }
        if let Ok(pattern)=element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
            if let Ok(selected)=pattern.CurrentIsSelected() { return Some(selected.as_bool()); }
        }
        let aria=element.CurrentAriaProperties().map(|v|v.to_string()).unwrap_or_default();
        if aria.split(';').any(|p|p.trim()=="checked=true") { return Some(true); }
        if aria.split(';').any(|p|p.trim()=="checked=false") { return Some(false); }
        let role=element.CurrentControlType().ok();
        if role==Some(UIA_RadioButtonControlTypeId) || role==Some(UIA_CheckBoxControlTypeId) {
            return element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
                .and_then(|p|p.CurrentState()).ok().map(|state|state&0x10!=0);
        }
        None
    }

    fn scan(&self) -> Result<Scan,String> {
        self.validate_window()?;
        unsafe {
            let root=self.api.ElementFromHandle(self.surface.hwnd()).map_err(|_|"UIA_ROOT_UNAVAILABLE")?;
            let condition=self.api.CreateTrueCondition().map_err(|_|"UIA_UNAVAILABLE")?;
            let nodes=root.FindAll(TreeScope_Descendants,&condition).map_err(|_|"UIA_TREE_UNAVAILABLE")?;
            let length=nodes.Length().map_err(|_|"UIA_TREE_UNAVAILABLE")?;
            let mut app_verified=false;
            let mut h5=false;
            let mut candidates=Vec::new();
            let mut home=std::collections::BTreeSet::new();
            let mut hidden_stadium=None;
            let mut document=None;
            let mut all_controls=Vec::new();
            let mut scroll_container=None;
            let mut login_marker=false;let mut authorization_marker=false;
            use std::hash::{Hash,Hasher};
            let mut geometry=std::collections::hash_map::DefaultHasher::new();
            let root_rect=root.CurrentBoundingRectangle().unwrap_or_default();
            for i in 0..length.min(MAX_NODES) {
                let Ok(element)=nodes.GetElement(i) else { continue; };
                let control_type=element.CurrentControlType().ok();
                let visible=Self::visible(&element) && element.CurrentBoundingRectangle().map(|r| {
                    if control_type==Some(UIA_DocumentControlTypeId) {
                        return r.left<root_rect.right && r.right>root_rect.left && r.top<root_rect.bottom && r.bottom>root_rect.top;
                    }
                    let x=r.left+(r.right-r.left)/2;let y=r.top+(r.bottom-r.top)/2;
                    x>=root_rect.left && x<root_rect.right && y>=root_rect.top && y<root_rect.bottom
                }).unwrap_or(false);
                if visible {
                    if let Ok(rect)=element.CurrentBoundingRectangle() {
                        (rect.left-root_rect.left,rect.top-root_rect.top,rect.right-root_rect.left,rect.bottom-root_rect.top,
                            control_type.map(|t|t.0)).hash(&mut geometry);
                    }
                }
                if scroll_container.is_none() {
                    if let Ok(pattern)=element.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId) {
                        if pattern.CurrentVerticallyScrollable().map(|v|v.as_bool()).unwrap_or(false) && self.in_app(&element) {scroll_container=Some(pattern);}
                    }
                }
                if control_type==Some(UIA_DocumentControlTypeId) {
                    let value=Self::value(&element);
                    if visible && value.starts_with(APP_ORIGIN) {app_verified=true;document=Some(element.clone());}
                    h5|=visible && value.starts_with(H5_PREFIX) && self.in_app(&element);
                    continue;
                }
                let name=element.CurrentName().map(|v|v.to_string()).unwrap_or_default();
                if matches!(name.trim(),"统一身份认证"|"微信授权"|"阅读并同意以下协议") && self.in_app(&element) {
                    login_marker|=name.trim()=="统一身份认证";
                    authorization_marker|=matches!(name.trim(),"微信授权"|"阅读并同意以下协议");
                }
                let home_label=matches!(name.trim(),"我的课程"|"阳光长跑"|"体质测试"|"教学视频");
                if (home_label || (!visible && name.trim()=="场馆预约")) && self.in_app(&element) {
                    // Keep recognizing the same home when its top row scrolls offscreen.
                    if home_label {home.insert(name.trim().to_owned());}
                    if !visible && name.trim()=="场馆预约" {hidden_stadium=Some(element.clone());}
                }
                let control=match name.trim() {
                    "立即登录"=>Some(Control::Login), "授权登录"=>Some(Control::Authorize),
                    "场馆预约"=>Some(Control::Stadium),
                    text if (text.starts_with("阅读并同意") || text.is_empty()) &&
                        (control_type==Some(UIA_RadioButtonControlTypeId) || control_type==Some(UIA_CheckBoxControlTypeId))=>Some(Control::Agreement),
                    _=>None,
                };
                let Some(control)=control else { continue; };
                if !self.in_app(&element) { continue; }
                let enabled=element.CurrentIsEnabled().map(|v|v.as_bool()).unwrap_or(false);
                let selected=if control==Control::Agreement {Self::selected(&element)} else {None};
                all_controls.push((control,enabled,selected,element.clone(),visible));
                if !visible {continue;}
                let legacy_action=element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
                    .and_then(|p|p.CurrentDefaultAction()).map(|v|!v.is_empty()).unwrap_or(false);
                let evidence=ControlEvidence{control,enabled,selected,
                    invoke:element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).is_ok(),
                    toggle:element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId).is_ok(),
                    select:element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId).is_ok(),legacy_action,
                    element_bounds_available:Self::visible(&element)};
                candidates.push(Candidate{control,element,evidence});
            }
            let controls=all_controls.iter().map(|(c,enabled,selected,_,_)|(*c,*enabled,*selected)).collect::<Vec<_>>();
            let desired=choose(h5,&controls);
            let hidden_target=if let Decision::Activate(control)=desired {
                if all_controls.iter().any(|(c,enabled,_,_,visible)|*c==control && *enabled && *visible) {None}
                else {all_controls.iter().find(|(c,_,_,_,visible)|*c==control && !visible).map(|(_,_,_,e,_)|e.clone())}
            } else {None};
            let decision=if app_verified && length<=MAX_NODES {
                if hidden_target.is_some() {Decision::Wait} else {desired}
            } else {Decision::Unsupported};
            let home_page=app_verified && length<=MAX_NODES && !h5
                && !all_controls.iter().any(|(c,_,_,_,_)|*c!=Control::Stadium)
                && (home.len()>=2 || hidden_stadium.is_some() || controls.iter().any(|(c,_,_)|*c==Control::Stadium));
            let search_page=if !app_verified || length>MAX_NODES || h5 {None}
                else if authorization_marker || controls.iter().any(|(c,_,_)|matches!(c,Control::Authorize|Control::Agreement)) {Some(SearchPage::Authorization)}
                else if login_marker || controls.iter().any(|(c,_,_)|*c==Control::Login) {Some(SearchPage::Login)}
                else if home_page {Some(SearchPage::Home)} else {None};
            if let (Some(target),Some(document))=(&hidden_target,&document) {
                let walker=self.api.RawViewWalker().map_err(|_|"UIA_TREE_UNAVAILABLE")?;
                let mut current=target.clone();
                for _ in 0..32 {
                    if let Ok(pattern)=current.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId) {
                        if pattern.CurrentVerticallyScrollable().map(|v|v.as_bool()).unwrap_or(false) {scroll_container=Some(pattern);break;}
                    }
                    if self.api.CompareElements(&current,document).map(|v|v.as_bool()).unwrap_or(false) {break;}
                    let Ok(parent)=walker.GetParentElement(&current) else {break;};
                    if !self.in_app(&parent) {break;}
                    current=parent;
                }
            }
            let percent=scroll_container.as_ref().and_then(|p|p.CurrentVerticalScrollPercent().ok()).filter(|p|*p>=0.0);
            let next_control=match desired {Decision::Activate(c)=>Some(c),_=>None};
            let report=Observation{decision,target_app_verified:app_verified,home_page,search_page,next_control,
                at_top:percent.map(|p|p<=0.01),at_bottom:percent.map(|p|p>=99.99),viewport_mark:Some(geometry.finish()),nodes_examined:length.min(MAX_NODES),
                truncated:length>MAX_NODES,controls:candidates.iter().map(|c|ControlEvidence{
                    control:c.control,enabled:c.evidence.enabled,selected:c.evidence.selected,invoke:c.evidence.invoke,
                    toggle:c.evidence.toggle,select:c.evidence.select,legacy_action:c.evidence.legacy_action,
                    element_bounds_available:c.evidence.element_bounds_available}).collect()};
            Ok(Scan{report,candidates,document,scroll_container})
        }
    }

    pub(crate) fn observe(&self) -> Result<Observation,String> { self.scan().map(|s|s.report) }

    /// One small scroll on a recognized login/home page, never an unknown page or H5.
    pub(crate) fn scroll(&self,direction:ScrollDirection,active:&dyn Fn()->bool)->Result<Option<&'static str>,String> {
        if !active() {return Err("已取消".into());}
        let _dpi=DpiGuard::enter();
        crate::wechat_window::foreground_surface(&self.surface)?;
        crate::wechat_window::portrait_surface(&self.surface,active)?;
        let scan=self.scan()?;
        if scan.report.search_page.is_none() || scan.report.decision==Decision::Handoff
            || (direction==ScrollDirection::Down && matches!(scan.report.decision,Decision::Activate(_))) {return Ok(None);}
        let search_page=scan.report.search_page;
        self.validate_window()?;
        unsafe {
            if GetForegroundWindow()!=self.surface.hwnd() {return Ok(None);}
            let Some(document)=scan.document else {return Ok(None);};
            if let Some(pattern)=scan.scroll_container {
                if !active() {return Err("已取消".into());}
                if let (Ok(percent),Ok(view_size))=(pattern.CurrentVerticalScrollPercent(),pattern.CurrentVerticalViewSize()) {
                    if percent>=0.0 && view_size>0.0 && view_size<100.0 {
                        // Half a viewport leaves overlap so a small window does not skip a button.
                        let delta=50.0*view_size/(100.0-view_size);
                        let target=(percent+if direction==ScrollDirection::Down {delta} else {-delta}).clamp(0.0,100.0);
                        if !active() {return Err("已取消".into());}
                        return pattern.SetScrollPercent(-1.0,target).map(|_|Some("uia_scroll_percent"))
                            .map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN".into());
                    }
                }
                if !active() {return Err("已取消".into());}
                return pattern.Scroll(ScrollAmount_NoAmount,match direction {ScrollDirection::Down=>ScrollAmount_SmallIncrement,ScrollDirection::Up=>ScrollAmount_SmallDecrement})
                    .map(|_|Some("uia_scroll_pattern")).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN".into());
            }
            let mut rect=document.CurrentBoundingRectangle().map_err(|_|"UIA_ELEMENT_CHANGED")?;
            let bounds=self.api.ElementFromHandle(self.surface.hwnd()).and_then(|e|e.CurrentBoundingRectangle()).map_err(|_|"UIA_WINDOW_CHANGED")?;
            rect.left=rect.left.max(bounds.left);rect.top=rect.top.max(bounds.top);
            rect.right=rect.right.min(bounds.right);rect.bottom=rect.bottom.min(bounds.bottom);
            if rect.right<=rect.left || rect.bottom<=rect.top {return Ok(None);}
            let point=POINT{x:rect.left+(rect.right-rect.left)/2,y:rect.top+(rect.bottom-rect.top)/2};
            if !crate::login_overlay::target_uncovered_at(point,self.surface.hwnd()) {return Ok(None);}
            let mut receiver=self.surface.hwnd();
            for _ in 0..16 {
                let mut local=point;
                if !ScreenToClient(receiver,&mut local).as_bool() {return Ok(None);}
                let child=ChildWindowFromPointEx(receiver,local,CWP_SKIPINVISIBLE|CWP_SKIPDISABLED);
                if child.0.is_null() || child==receiver {break;}
                receiver=child;
            }
            let mut pid=0;GetWindowThreadProcessId(receiver,Some(&mut pid));
            if pid!=self.surface.owner.pid || GetAncestor(receiver,GA_ROOT)!=self.surface.hwnd() {return Ok(None);}
            let fresh=self.scan()?;
            if fresh.report.search_page!=search_page || fresh.report.decision==Decision::Handoff
                || (direction==ScrollDirection::Down && matches!(fresh.report.decision,Decision::Activate(_))) {return Ok(None);}
            self.validate_window()?;
            if GetForegroundWindow()!=self.surface.hwnd() || !crate::login_overlay::target_uncovered_at(point,self.surface.hwnd()) {return Ok(None);}
            if !(-32768..=32767).contains(&point.x) || !(-32768..=32767).contains(&point.y) {return Ok(None);}
            let position=LPARAM((((point.y as u16 as u32)<<16)|(point.x as u16 as u32)) as isize);
            // WM_MOUSEWHEEL uses screen coordinates and signed delta in the high word.
            if !active() {return Err("已取消".into());}
            PostMessageW(Some(receiver),windows::Win32::UI::WindowsAndMessaging::WM_MOUSEWHEEL,
                WPARAM((((match direction {ScrollDirection::Down=>-120i16,ScrollDirection::Up=>120i16}) as u16 as u32)<<16) as usize),position).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN")?;
            Ok(Some("uia_scroll_message"))
        }
    }

    pub(crate) fn refresh(&mut self,active:&dyn Fn()->bool)->Result<(),String> {
        if !active() {return Err("已取消".into());}
        crate::wechat_window::foreground_surface(&self.surface)?;
        crate::wechat_window::portrait_surface(&self.surface,active)?;
        // Recreate the client on this initialized thread without rebinding the surface.
        unsafe {
            let api:IUIAutomation=CoCreateInstance(&CUIAutomation8,None,CLSCTX_INPROC_SERVER).map_err(|_|"UIA_UNAVAILABLE")?;
            if let Ok(timed)=api.cast::<IUIAutomation2>() {let _=timed.SetConnectionTimeout(1000);let _=timed.SetTransactionTimeout(2000);}
            self.validate_window()?;
            self.api=api;
        }
        Ok(())
    }

    /// Re-resolve and recheck before one input. Coordinates, when necessary,
    /// come only from the live named UIA element, never OCR or a fixed layout.
    /// None means no input was attempted and another adapter may be used.
    pub(crate) fn activate(&self, control: Control,active:&dyn Fn()->bool) -> Result<Option<&'static str>,String> {
        if !active() {return Err("已取消".into());}
        let _dpi=DpiGuard::enter();
        crate::wechat_window::foreground_surface(&self.surface)?;
        crate::wechat_window::portrait_surface(&self.surface,active)?;
        let scan=self.scan()?;
        if scan.report.decision!=Decision::Activate(control) { return Ok(None); }
        let matching=scan.candidates.iter().filter(|c|c.control==control && c.evidence.enabled).collect::<Vec<_>>();
        if matching.len()!=1 { return Ok(None); }
        let candidate=matching[0];
        self.validate_window()?;
        unsafe {
            if GetForegroundWindow()!=self.surface.hwnd() { return Ok(None); }
            let element=&candidate.element;
            if !Self::visible(element) || !element.CurrentIsEnabled().map(|v|v.as_bool()).unwrap_or(false) { return Ok(None); }
            if control==Control::Agreement {
                if Self::selected(element)!=Some(false) { return Ok(None); }
                if candidate.evidence.select {
                    if !active() {return Err("已取消".into());}
                    return element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                        .and_then(|p|p.Select()).map(|_|Some("uia_select")).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN".into());
                }
                if candidate.evidence.toggle {
                    if !active() {return Err("已取消".into());}
                    return element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                        .and_then(|p|p.Toggle()).map(|_|Some("uia_toggle")).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN".into());
                }
            }
            if candidate.evidence.invoke {
                if !active() {return Err("已取消".into());}
                return element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                    .and_then(|p|p.Invoke()).map(|_|Some("uia_invoke")).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN".into());
            }
            if candidate.evidence.legacy_action {
                if !active() {return Err("已取消".into());}
                return element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
                    .and_then(|p|p.DoDefaultAction()).map(|_|Some("uia_default_action")).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN".into());
            }
            let rect=element.CurrentBoundingRectangle().map_err(|_|"UIA_ELEMENT_CHANGED")?;
            if rect.right<=rect.left || rect.bottom<=rect.top { return Ok(None); }
            let point=POINT{x:rect.left+(rect.right-rect.left)/2,y:rect.top+(rect.bottom-rect.top)/2};
            if control==Control::Agreement && Self::selected(element)!=Some(false) { return Ok(None); }
            self.validate_window()?;
            // Descend within the verified mini-program only.
            let mut receiver=self.surface.hwnd();
            for _ in 0..16 {
                let mut local=point;
                if !ScreenToClient(receiver,&mut local).as_bool() {return Ok(None);}
                let child=ChildWindowFromPointEx(receiver,local,CWP_SKIPINVISIBLE|CWP_SKIPDISABLED);
                if child.0.is_null() || child==receiver {break;}
                receiver=child;
            }
            let mut pid=0;GetWindowThreadProcessId(receiver,Some(&mut pid));
            if pid!=self.surface.owner.pid || GetAncestor(receiver,GA_ROOT)!=self.surface.hwnd() {return Ok(None);}
            // Re-resolve semantics before dispatch, including agreement state.
            let fresh=self.scan()?;
            if fresh.report.decision!=Decision::Activate(control) {return Ok(None);}
            let matching=fresh.candidates.iter().filter(|c|c.control==control && c.evidence.enabled).collect::<Vec<_>>();
            if matching.len()!=1 || !self.api.CompareElements(&matching[0].element,element).map(|v|v.as_bool()).unwrap_or(false) {return Ok(None);}
            let current_rect=matching[0].element.CurrentBoundingRectangle().map_err(|_|"UIA_ELEMENT_CHANGED")?;
            if (current_rect.left,current_rect.top,current_rect.right,current_rect.bottom)!=(rect.left,rect.top,rect.right,rect.bottom) {return Ok(None);}
            let mut client=point;
            if !ScreenToClient(receiver,&mut client).as_bool() || !(0..=32767).contains(&client.x) || !(0..=32767).contains(&client.y) {return Ok(None);}
            let position=LPARAM(((client.y as u32)<<16 | client.x as u32) as isize);
            if GetForegroundWindow()!=self.surface.hwnd() || !crate::login_overlay::target_uncovered_at(point,self.surface.hwnd()) {
                return Ok(None);
            }
            // Target only the current verified window. Never move the user's
            // pointer or inject desktop mouse input, even if navigation fails.
            if !active() {return Err("已取消".into());}
            PostMessageW(Some(receiver),WM_MOUSEMOVE,WPARAM(0),position).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN")?;
            PostMessageW(Some(receiver),WM_LBUTTONDOWN,WPARAM(1),position).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN")?;
            PostMessageW(Some(receiver),WM_LBUTTONUP,WPARAM(0),position).map_err(|_|"UIA_INPUT_OUTCOME_UNKNOWN")?;
            return Ok(Some("uia_window_message"));
        }
    }
}

/// Standalone experiment: no foregrounding, coordinate input, booking or scheduler.
/// Normal Session visibility/foreground protections remain unchanged.
pub(crate) fn probe_minimized()->Result<serde_json::Value,String> {
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic,IsZoomed,IsWindow,ShowWindow,SW_MINIMIZE,SW_RESTORE,SW_SHOWMAXIMIZED,GetWindowThreadProcessId};
    let mut surfaces=crate::wechat_window::discover_wechat_candidates()?;
    if surfaces.is_empty() {
        crate::wechat_window::launch_xmu_miniprogram(None)?;
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            surfaces=crate::wechat_window::discover_wechat_candidates()?;
            if !surfaces.is_empty() {break;}
        }
    }
    if surfaces.len()!=1 {return Err("UIA_TARGET_AMBIGUOUS_OR_MISSING".into());}
    let surface=surfaces.remove(0);
    let hwnd=surface.hwnd();
    let was_minimized=unsafe{IsIconic(hwnd).as_bool()};
    let was_maximized=unsafe{IsZoomed(hwnd).as_bool()};
    struct Restore(crate::wechat_window::NativeSurface,bool,bool);
    impl Drop for Restore {
        fn drop(&mut self) {unsafe {
            if IsWindow(Some(self.0.hwnd())).as_bool() {
                let _=ShowWindow(self.0.hwnd(),if self.1 {SW_MINIMIZE} else if self.2 {SW_SHOWMAXIMIZED} else {SW_RESTORE});
            }
        }}
    }
    let _restore=Restore(surface.clone(),was_minimized,was_maximized);
    unsafe {let _=ShowWindow(hwnd,SW_MINIMIZE);}
    std::thread::sleep(std::time::Duration::from_millis(800));
    if !unsafe{IsIconic(hwnd).as_bool()} {return Err("MINIMIZE_FAILED".into());}
    unsafe {
        let _=CoInitializeEx(None,COINIT_MULTITHREADED);
        let api:IUIAutomation=CoCreateInstance(&CUIAutomation8,None,CLSCTX_INPROC_SERVER).map_err(|_|"UIA_UNAVAILABLE")?;
        if let Ok(timed)=api.cast::<IUIAutomation2>() {let _=timed.SetConnectionTimeout(1000);let _=timed.SetTransactionTimeout(2000);}
        let session=Session{api,surface};
        let root=session.api.ElementFromHandle(hwnd).map_err(|_|"UIA_ROOT_UNAVAILABLE")?;
        let nodes=root.FindAll(TreeScope_Descendants,&session.api.CreateTrueCondition().map_err(|_|"UIA_UNAVAILABLE")?).map_err(|_|"UIA_TREE_UNAVAILABLE")?;
        let length=nodes.Length().map_err(|_|"UIA_TREE_UNAVAILABLE")?;
        let mut verified=false;let mut h5=false;let mut candidates=Vec::new();
        for i in 0..length.min(MAX_NODES) {
            let Ok(element)=nodes.GetElement(i) else {continue;};
            if element.CurrentControlType().ok()==Some(UIA_DocumentControlTypeId) {
                let value=Session::value(&element);
                verified|=value.starts_with(APP_ORIGIN);
                h5|=value.starts_with(H5_PREFIX) && session.in_app(&element);
            }
            if !session.in_app(&element) {continue;}
            let name=element.CurrentName().map(|v|v.to_string()).unwrap_or_default();
            let control=match name.trim() {"立即登录"=>Control::Login,"授权登录"=>Control::Authorize,"场馆预约"=>Control::Stadium,_=>continue};
            candidates.push((control,element));
        }
        let controls=candidates.iter().map(|(control,element)|(*control,element.CurrentIsEnabled().map(|v|v.as_bool()).unwrap_or(false),None)).collect::<Vec<_>>();
        let decision=if verified && length<=MAX_NODES {choose(h5,&controls)} else {Decision::Unsupported};
        let evidence=candidates.iter().map(|(control,element)|serde_json::json!({"control":control,
            "offscreen":element.CurrentIsOffscreen().map(|v|v.as_bool()).unwrap_or(true),
            "invoke":element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).is_ok(),
            "legacy_action":element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId).and_then(|p|p.CurrentDefaultAction()).map(|v|!v.is_empty()).unwrap_or(false)})).collect::<Vec<_>>();
        let mut method=None;let mut error=None;
        if let Decision::Activate(control)=decision {
            let matched=candidates.iter().filter(|(c,_)|*c==control).collect::<Vec<_>>();
            let mut pid=0;GetWindowThreadProcessId(hwnd,Some(&mut pid));
            if matched.len()==1 && pid==session.surface.owner.pid && IsIconic(hwnd).as_bool() {
                let element=&matched[0].1;
                if let Ok(pattern)=element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
                    method=Some("uia_invoke"); if pattern.Invoke().is_err() {error=Some("UIA_INPUT_OUTCOME_UNKNOWN");}
                } else if let Ok(pattern)=element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId) {
                    if pattern.CurrentDefaultAction().map(|v|!v.is_empty()).unwrap_or(false) {
                        method=Some("uia_default_action");if pattern.DoDefaultAction().is_err() {error=Some("UIA_INPUT_OUTCOME_UNKNOWN");}
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
        Ok(serde_json::json!({"ok":true,"scheduler_started":false,"booking_actions_available":false,
            "coordinate_input_used":false,"window_minimized_before":true,"window_minimized_after":IsIconic(hwnd).as_bool(),
            "nodes_examined":length.min(MAX_NODES),"truncated":length>MAX_NODES,"target_app_verified":verified,
            "decision":decision,"controls":evidence,"attempted_method":method,"action_error":error}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorization_requires_known_agreement_state_and_does_not_uncheck() {
        let controls=|state|vec![(Control::Authorize,true,None),(Control::Agreement,true,state)];
        assert_eq!(choose(false,&controls(Some(false))),Decision::Activate(Control::Agreement));
        assert_eq!(choose(false,&controls(Some(true))),Decision::Activate(Control::Authorize));
        assert_eq!(choose(false,&controls(None)),Decision::Unsupported);
        assert_eq!(choose(false,&[(Control::Authorize,false,None)]),Decision::Unsupported);
        assert_eq!(choose(false,&[(Control::Authorize,true,None)]),Decision::Unsupported,"missing agreement is not consent");
    }
    #[test]
    fn stadium_handoff_has_priority_and_ambiguous_agreements_stop() {
        assert_eq!(choose(true,&[(Control::Authorize,true,None)]),Decision::Handoff);
        assert_eq!(choose(false,&[(Control::Authorize,true,None),(Control::Agreement,true,Some(false)),(Control::Agreement,true,Some(false))]),Decision::Unsupported);
    }
}
