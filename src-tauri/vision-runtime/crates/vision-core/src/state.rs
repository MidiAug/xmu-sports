use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    Unknown,
    WechatMain,
    WechatDiscover,
    WechatMpPanel,
    WechatMpSearch,
    /// 厦大小程序登录门：立即登录 / 统一身份认证
    XmuLoginGate,
    /// 微信授权步骤：勾选协议 → 授权登录
    XmuWxAuthorize,
    /// 授权提交后中间「等待中」动画
    XmuAuthWaiting,
    Home,
    VenueSportList,
    VenueList,
    VenueDetail,
    SelectSlot,
    ConfirmOrder,
    MyReservations,
    OrderDetail,
    CancelModal,
    Success,
    /// 点开场馆预约后盖在运动列表上的须知，底部只有「我知道了」。
    BookingNotice,
}

/// PC 小程序窗。不是微信壳，也不是本程序自己的「预约助手」窗口。
pub fn title_is_xmu_miniprogram(title: &str) -> bool {
    if title.contains("预约助手") {
        return false;
    }
    title.contains("厦大体育") || title.contains("步道乐跑") || title.contains("厦门大学")
}

impl Page {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::WechatMain => "wechat_main",
            Self::WechatDiscover => "wechat_discover",
            Self::WechatMpPanel => "wechat_mp_panel",
            Self::WechatMpSearch => "wechat_mp_search",
            Self::XmuLoginGate => "xmu_login_gate",
            Self::XmuWxAuthorize => "xmu_wx_authorize",
            Self::XmuAuthWaiting => "xmu_auth_waiting",
            Self::Home => "home",
            Self::VenueSportList => "venue_sport_list",
            Self::VenueList => "venue_list",
            Self::VenueDetail => "venue_detail",
            Self::SelectSlot => "select_slot",
            Self::ConfirmOrder => "confirm_order",
            Self::MyReservations => "my_reservations",
            Self::OrderDetail => "order_detail",
            Self::CancelModal => "cancel_modal",
            Self::Success => "success",
            Self::BookingNotice => "booking_notice",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Overlay {
    None,
    Keyboard,
    Toast,
    Modal,
    Notice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotCta {
    NeedSelect,
    Ready,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptchaPhase {
    IdleEmpty,
    KbOpenEmpty,
    KbOpenPartial,
    KbOpenFull,
    IdleFull,
    ToastNeedInput,
    ToastError,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub page: Page,
    pub title_hint: String,
    pub slot_cta: SlotCta,
    pub captcha: CaptchaPhase,
    pub keyboard_open: bool,
    pub input_chars: String,
    pub has_mint_slots: bool,
    pub toast_error: bool,
    pub toast_need_input: bool,
    pub overlay: Overlay,
    pub evidence: Vec<String>,
    pub confidence: f32,
    /// Body fingerprint for Guard re-check (not title alone).
    pub body_fingerprint: String,
}

impl Default for UiSnapshot {
    fn default() -> Self {
        Self {
            page: Page::Unknown,
            title_hint: String::new(),
            slot_cta: SlotCta::Unknown,
            captcha: CaptchaPhase::Unknown,
            keyboard_open: false,
            input_chars: String::new(),
            has_mint_slots: false,
            toast_error: false,
            toast_need_input: false,
            overlay: Overlay::None,
            evidence: Vec::new(),
            confidence: 0.0,
            body_fingerprint: String::new(),
        }
    }
}

pub fn can_submit_order(snap: &UiSnapshot) -> bool {
    snap.page == Page::ConfirmOrder
        && snap.captcha == CaptchaPhase::IdleFull
        && !snap.keyboard_open
        && snap.input_chars.len() >= 4
        && !snap.toast_error
        && !snap.toast_need_input
}

pub fn can_confirm_slot(snap: &UiSnapshot) -> bool {
    snap.page == Page::SelectSlot && snap.slot_cta == SlotCta::Ready
}
