use axum::Router;
use eink_bridge::app::{AppState, build_app};
use std::path::PathBuf;

pub fn test_app(state_dir: PathBuf) -> Router {
    let state = AppState::new(state_dir);
    build_app(state)
}
