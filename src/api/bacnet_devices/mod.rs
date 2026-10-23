mod devices;
mod discover;
mod objects;

pub use devices::*;
pub use discover::*;
pub use objects::*;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::models::{
    BacnetDeviceConfig, BacnetObjectConfig, CreateBacnetDeviceRequest, CreateBacnetObjectRequest,
    UpdateBacnetDeviceRequest,
};
