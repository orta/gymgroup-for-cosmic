// SPDX-License-Identifier: MPL-2.0

use crate::api::GymClass;
use icalendar::{Calendar, Component, Event, EventLike, EventStatus};
use std::fs;
use std::path::PathBuf;

const CALENDAR_ID: &str = "gymgroup-classes";
const CALENDAR_NAME: &str = "The Gym Group";
const CALENDAR_COLOR: &str = "#ff5733";
/// Sync booked classes to a dedicated Evolution Data Server calendar.
/// This makes them appear automatically in GNOME Calendar.
pub fn sync_calendar(classes: &[GymClass], gym_name: Option<&str>) {
    if let Err(e) = sync_calendar_inner(classes, gym_name) {
        eprintln!("Calendar sync failed: {e}");
    }
}

fn sync_calendar_inner(classes: &[GymClass], gym_name: Option<&str>) -> Result<(), String> {
    ensure_eds_source()?;

    let booked: Vec<&GymClass> = classes
        .iter()
        .filter(|c| c.booked == Some(true) && c.cancelled != Some(true))
        .collect();

    let location = gym_name
        .map(|name| format!("The Gym Group \u{2014} {name}"))
        .unwrap_or_else(|| "The Gym Group".to_string());

    let cal_dir = calendar_dir()?;
    fs::create_dir_all(&cal_dir).map_err(|e| format!("Failed to create calendar dir: {e}"))?;

    // Build a single calendar.ics with all booked events.
    // Include EDS-compatible headers so Evolution detects changes.
    let revision = chrono::Utc::now()
        .format("%Y%m%dT%H%M%SZ")
        .to_string();

    let mut cal = Calendar::new();
    cal.append_property(("PRODID", "-//Ximian//NONSGML Evolution Calendar//EN"));
    cal.append_property(("X-EVOLUTION-DATA-REVISION", &*format!("{revision}(0)")));

    for class in &booked {
        if let Some(event) = class_to_event(class, &location) {
            cal.push(event);
        }
    }

    let cal_file = cal_dir.join("calendar.ics");
    fs::write(&cal_file, cal.done().to_string())
        .map_err(|e| format!("Failed to write calendar.ics: {e}"))?;

    Ok(())
}

fn class_to_event(class: &GymClass, location: &str) -> Option<Event> {
    let id = class.id.as_ref()?;
    let start_ms = class.start_date_time?;
    let end_ms = class.end_date_time?;

    let start = chrono::DateTime::from_timestamp_millis(start_ms)?;
    let end = chrono::DateTime::from_timestamp_millis(end_ms)?;

    let name = class.display_name().to_string();
    let instructor = class.instructor_name();

    let summary = if instructor.is_empty() {
        format!("Gym: {name}")
    } else {
        format!("Gym: {name} — {instructor}")
    };

    let mut description = name;
    if !instructor.is_empty() {
        description.push_str(&format!("\nInstructor: {instructor}"));
    }
    let spots = class.spots_text();
    if !spots.is_empty() {
        description.push_str(&format!("\n{spots}"));
    }

    let mut event = Event::new();
    event
        .uid(&format!("gym-class-{id}"))
        .summary(&summary)
        .description(&description)
        .location(location)
        .starts(start)
        .ends(end)
        .timestamp(chrono::Utc::now())
        .status(EventStatus::Confirmed);

    Some(event.done())
}

fn xdg_config_home() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".config")
        })
}

fn xdg_data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".local").join("share")
        })
}

fn eds_sources_dir() -> Result<PathBuf, String> {
    Ok(xdg_config_home().join("evolution").join("sources"))
}

fn calendar_dir() -> Result<PathBuf, String> {
    Ok(xdg_data_home()
        .join("evolution")
        .join("calendar")
        .join(CALENDAR_ID))
}

/// Create the EDS source file if it doesn't exist yet.
/// This registers the calendar so GNOME Calendar shows it.
fn ensure_eds_source() -> Result<(), String> {
    let sources_dir = eds_sources_dir()?;
    let source_file = sources_dir.join(format!("{CALENDAR_ID}.source"));

    if source_file.exists() {
        return Ok(());
    }

    fs::create_dir_all(&sources_dir)
        .map_err(|e| format!("Failed to create sources dir: {e}"))?;

    let content = format!(
        "\
[Data Source]
DisplayName={CALENDAR_NAME}
Enabled=true
Parent=local-stub

[Calendar]
BackendName=local
Color={CALENDAR_COLOR}
Selected=true
Order=100

[Offline]
StaySynchronized=true

[Refresh]
Enabled=true
IntervalMinutes=30
EnabledOnMeteredNetwork=true
"
    );

    fs::write(&source_file, content)
        .map_err(|e| format!("Failed to write EDS source: {e}"))?;

    Ok(())
}
