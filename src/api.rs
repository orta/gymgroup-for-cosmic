// SPDX-License-Identifier: MPL-2.0

use chrono::{Local, TimeZone, Utc};
use reqwest::Client;
use serde::Deserialize;

const BASE_URL: &str = "https://thegymgroup.netpulse.com/np";


pub fn create_client() -> Client {
    use reqwest::header::{HeaderMap, HeaderValue};

    let mut headers = HeaderMap::new();
    headers.insert("accept", HeaderValue::from_static("application/json"));
    headers.insert("x-np-api-version", HeaderValue::from_static("1.5"));
    headers.insert("x-np-app-version", HeaderValue::from_static("9999"));
    headers.insert(
        "x-np-user-agent",
        HeaderValue::from_static(
            "clientType=MOBILE_DEVICE; devicePlatform=ANDROID; deviceUid=; \
             applicationName=The Gym Group; applicationVersion=5.0; applicationVersionCode=38",
        ),
    );

    Client::builder()
        .cookie_store(true)
        .user_agent("okhttp/3.12.3")
        .default_headers(headers)
        .build()
        .expect("Failed to create HTTP client")
}

// --- Login ---

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub uuid: String,
    pub home_club_uuid: String,
    pub home_club_name: String,
}

pub async fn login(client: &Client, username: &str, pin: &str) -> Result<LoginResponse, String> {
    let resp = client
        .post(format!("{BASE_URL}/exerciser/login"))
        .form(&[("username", username), ("password", pin)])
        .send()
        .await
        .map_err(|e| format!("Login failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Login failed ({status}): {body}"));
    }

    resp.json().await.map_err(|e| format!("Parse error: {e}"))
}

// --- Classes ---

/// Wrapper: each item in the array has a `brief` field
#[derive(Debug, Clone, Deserialize)]
pub struct ClassEntry {
    pub brief: GymClass,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GymClass {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub start_date_time: Option<i64>,
    #[serde(default)]
    pub end_date_time: Option<i64>,
    #[serde(default)]
    pub max_capacity: Option<i32>,
    #[serde(default)]
    pub total_booked: Option<i32>,
    #[serde(default)]
    pub instructor: Option<Instructor>,
    #[serde(default)]
    pub activity: Option<Activity>,
    #[serde(default)]
    pub reservable: Option<bool>,
    #[serde(default)]
    pub booked: Option<bool>,
    #[serde(default)]
    pub cancelled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Instructor {
    #[serde(default)]
    pub full_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    #[serde(default)]
    pub description: Option<String>,
}

impl GymClass {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Unknown Class")
    }

    pub fn instructor_name(&self) -> &str {
        self.instructor
            .as_ref()
            .and_then(|i| i.full_name.as_deref())
            .filter(|n| !n.is_empty())
            .unwrap_or("")
    }

    pub fn formatted_time(&self) -> String {
        match self.start_date_time {
            Some(ms) => {
                let dt = chrono::DateTime::from_timestamp_millis(ms)
                    .unwrap_or_default()
                    .with_timezone(&Local);
                dt.format("%H:%M").to_string()
            }
            None => "TBD".to_string(),
        }
    }

    pub fn duration_minutes(&self) -> Option<i64> {
        match (self.start_date_time, self.end_date_time) {
            (Some(start), Some(end)) => Some((end - start) / 60_000),
            _ => None,
        }
    }

    pub fn class_id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }

    pub fn spots_text(&self) -> String {
        match (self.total_booked, self.max_capacity) {
            (Some(booked), Some(max)) => {
                let available = max - booked;
                if available <= 0 {
                    "Full".to_string()
                } else {
                    format!("{available}/{max} free")
                }
            }
            _ => String::new(),
        }
    }

    pub fn is_full(&self) -> bool {
        match (self.total_booked, self.max_capacity) {
            (Some(booked), Some(max)) => booked >= max,
            _ => false,
        }
    }
}

pub async fn get_classes(
    client: &Client,
    gym_uuid: &str,
    user_uuid: &str,
) -> Result<Vec<GymClass>, String> {
    let now = Utc::now().timestamp_millis();
    let week_later = now + 7 * 24 * 60 * 60 * 1000;

    let resp = client
        .get(format!("{BASE_URL}/company/{gym_uuid}/classes"))
        .query(&[
            ("startDateTime", now.to_string()),
            ("endDateTime", week_later.to_string()),
            ("exerciserUuid", user_uuid.to_string()),
        ])
        .send()
        .await
        .map_err(|e| format!("Failed to fetch classes: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Classes request failed ({status}): {body}"));
    }

    let entries: Vec<ClassEntry> = resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;

    Ok(entries.into_iter().map(|e| e.brief).collect())
}

// --- Busyness ---

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Busyness {
    #[serde(default)]
    pub gym_location_name: Option<String>,
    #[serde(default)]
    pub current_capacity: Option<i32>,
    #[serde(default)]
    pub current_percentage: Option<f64>,
}

pub async fn get_busyness(
    client: &Client,
    user_uuid: &str,
    gym_uuid: &str,
) -> Result<Busyness, String> {
    let resp = client
        .get(format!(
            "{BASE_URL}/thegymgroup/v1.0/exerciser/{user_uuid}/gym-busyness"
        ))
        .query(&[("gymLocationId", gym_uuid)])
        .send()
        .await
        .map_err(|e| format!("Failed to fetch busyness: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Busyness request failed ({status}): {body}"));
    }

    resp.json().await.map_err(|e| format!("Parse error: {e}"))
}

// --- Check-in History ---

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CheckIn {
    #[serde(default)]
    pub gym_location_name: Option<String>,
    #[serde(default)]
    pub gym_location_address: Option<String>,
    #[serde(default)]
    pub check_in_date: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub duration: Option<i64>,
}

impl CheckIn {
    pub fn duration_minutes(&self) -> Option<i64> {
        self.duration.map(|ms| ms / 60_000)
    }

    pub fn formatted_date(&self) -> String {
        match &self.check_in_date {
            Some(date_str) => {
                if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S")
                {
                    let local = Local
                        .from_local_datetime(&dt)
                        .single()
                        .unwrap_or_default();
                    local.format("%a %d %b %Y %H:%M").to_string()
                } else {
                    date_str.clone()
                }
            }
            None => "Unknown date".to_string(),
        }
    }
}

pub async fn get_check_in_history(
    client: &Client,
    user_uuid: &str,
) -> Result<Vec<CheckIn>, String> {
    let end = Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    let resp = client
        .get(format!(
            "{BASE_URL}/exercisers/{user_uuid}/check-ins/history"
        ))
        .query(&[
            ("startDate", "2020-01-01T00:00:00".to_string()),
            ("endDate", end),
        ])
        .send()
        .await
        .map_err(|e| format!("Failed to fetch history: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("History request failed ({status}): {body}"));
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        #[serde(default)]
        check_ins: Vec<CheckIn>,
    }

    let response: Response = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;
    Ok(response.check_ins)
}

// --- Booking ---

pub async fn book_class(
    client: &Client,
    gym_uuid: &str,
    class_id: &str,
    user_uuid: &str,
) -> Result<(), String> {
    let resp = client
        .post(format!(
            "{BASE_URL}/company/{gym_uuid}/class/{class_id}/addExerciser"
        ))
        .form(&[("exerciserUuid", user_uuid)])
        .send()
        .await
        .map_err(|e| format!("Failed to book class: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Booking failed ({status}): {body}"));
    }

    Ok(())
}

pub async fn cancel_class(
    client: &Client,
    gym_uuid: &str,
    class_id: &str,
    user_uuid: &str,
) -> Result<(), String> {
    let resp = client
        .post(format!(
            "{BASE_URL}/company/{gym_uuid}/class/{class_id}/removeExerciser"
        ))
        .form(&[("exerciserUuid", user_uuid)])
        .send()
        .await
        .map_err(|e| format!("Failed to cancel class: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Cancellation failed ({status}): {body}"));
    }

    Ok(())
}
