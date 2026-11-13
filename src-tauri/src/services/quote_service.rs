use std::sync::{atomic::AtomicBool, Mutex, OnceLock};

use tauri::menu::CheckMenuItem;
use tauri::Wry;

use crate::models::GoldQuote;
use crate::providers::huilvsudi::HuilvsudiCmbProvider;

/// 全局应用状态（由 Tauri manage）
pub struct AppState {
    pub db: Mutex<crate::storage::db::Db>,
    /// 最近一次有效行情
    pub latest_quote: Mutex<Option<GoldQuote>>,
    /// 最近一次行情获取错误（用于前端展示）
    pub last_error: Mutex<Option<String>>,
    pub provider: HuilvsudiCmbProvider,
    /// 行情请求互斥闸门：同一时刻只允许一个请求进行
    pub fetch_gate: tokio::sync::Mutex<()>,
    pub alert_recovery_pending: AtomicBool,
    pub stabilization_recovery_pending: AtomicBool,
    /// 托盘菜单「悬浮行情条」勾选项（供任意入口同步勾选状态）
    pub hud_check: OnceLock<CheckMenuItem<Wry>>,
    /// 托盘菜单「菜单栏价格文字」勾选项
    pub title_check: OnceLock<CheckMenuItem<Wry>>,
}

impl AppState {
    pub fn new(db: crate::storage::db::Db) -> Self {
        Self {
            db: Mutex::new(db),
            latest_quote: Mutex::new(None),
            last_error: Mutex::new(None),
            provider: HuilvsudiCmbProvider::new(),
            fetch_gate: tokio::sync::Mutex::new(()),
            alert_recovery_pending: AtomicBool::new(false),
            stabilization_recovery_pending: AtomicBool::new(false),
            hud_check: OnceLock::new(),
            title_check: OnceLock::new(),
        }
    }

    pub fn set_quote(&self, quote: GoldQuote) {
        *self.latest_quote.lock().unwrap() = Some(quote);
        *self.last_error.lock().unwrap() = None;
    }

    pub fn set_error(&self, err: &str) {
        *self.last_error.lock().unwrap() = Some(err.to_string());
    }
}
