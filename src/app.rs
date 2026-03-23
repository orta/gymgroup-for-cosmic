// SPDX-License-Identifier: MPL-2.0

use crate::api::{self, Busyness, CheckIn, GymClass};
use crate::config::{Config, PlusOneConfig};
use crate::fl;
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use cosmic::iced_core::widget::Id as WidgetId;
use cosmic::{prelude::*, Task};
use std::collections::{HashMap, HashSet};

fn note_input_id() -> WidgetId {
    WidgetId::new("class-note-input")
}

#[derive(Debug, Clone)]
struct MonitorInfo {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/icon.svg");

/// The application model stores app-specific state.
pub struct AppModel {
    /// Application state managed by the COSMIC runtime.
    core: cosmic::Core,
    /// Display a context drawer with the designated page if defined.
    context_page: ContextPage,
    /// The about page for this app.
    about: About,
    /// Contains items assigned to the nav bar panel.
    nav: nav_bar::Model,
    /// Key bindings for the application's menu bar.
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    /// Configuration data that persists between application runs.
    config: Config,
    /// Handle for writing config to disk.
    config_handler: Option<cosmic_config::Config>,

    // --- API state ---
    client: reqwest::Client,
    user_uuid: Option<String>,
    gym_uuid: Option<String>,
    gym_name: Option<String>,
    login_error: Option<String>,

    // --- Login form state (transient) ---
    login_username: String,
    login_pin: String,

    // --- Data ---
    classes: Vec<GymClass>,
    classes_error: Option<String>,
    busyness: Option<Busyness>,
    check_ins: Vec<CheckIn>,
    history_error: Option<String>,
    loading: bool,

    // --- Reminders ---
    notified_classes: HashSet<String>,
    gym_warning: Option<String>,
    warning_window_ids: Vec<cosmic::iced::window::Id>,

    // --- Class notes ---
    editing_note_key: String,
    editing_note_text: String,

    // --- Plus Ones (indexed same as config.plus_ones) ---
    plus_one_clients: Vec<reqwest::Client>,
    plus_one_uuids: Vec<Option<String>>,
    /// Maps class_id → names of Plus Ones who are booked into that class.
    plus_one_booked_by: HashMap<String, Vec<String>>,

    // Plus One management form state
    plus_one_form_name: String,
    plus_one_form_username: String,
    plus_one_form_pin: String,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    // Auth
    LoggedIn(Result<api::LoginResponse, String>),

    // Login form
    LoginUsernameChanged(String),
    LoginPinChanged(String),
    SubmitLogin,
    Logout,

    // Data loading
    ClassesLoaded(Result<Vec<GymClass>, String>),
    BusynessLoaded(Result<Busyness, String>),
    HistoryLoaded(Result<Vec<CheckIn>, String>),

    // Booking
    BookClass(String),
    BookingResult(Result<String, String>),
    CancelClass(String),
    CancelResult(Result<String, String>),

    Refresh,

    // Class notes
    OpenClassNote(String),
    ClassNoteChanged(String),
    SaveClassNote,

    // Reminders
    CheckReminders,

    // Window control
    CloseRequested,
    WarningWindowOpened(cosmic::iced::window::Id),
    WarningWindowClosed,

    // Plus Ones
    PlusOneLoggedIn(usize, Result<api::LoginResponse, String>),
    PlusOneClassesLoaded(usize, Result<Vec<GymClass>, String>),
    AddPlusOne,
    RemovePlusOne(usize),
    PlusOneFormNameChanged(String),
    PlusOneFormUsernameChanged(String),
    PlusOneFormPinChanged(String),

    // UI
    LaunchUrl(String),
    ToggleContextPage(ContextPage),
    UpdateConfig(Config),
}

/// Create a COSMIC application from the app model
impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.github.orta.gym";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let mut nav = nav_bar::Model::default();

        nav.insert()
            .text(fl!("nav-classes"))
            .data::<Page>(Page::Classes)
            .icon(icon::from_name("x-office-calendar-symbolic"))
            .activate();

        nav.insert()
            .text(fl!("nav-history"))
            .data::<Page>(Page::History)
            .icon(icon::from_name("document-open-recent-symbolic"));

        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license(env!("CARGO_PKG_LICENSE"));

        let client = api::create_client();

        let (config, config_handler) = match cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
        {
            Ok(config_handler) => {
                let config = match Config::get_entry(&config_handler) {
                    Ok(config) => config,
                    Err((_errors, config)) => config,
                };
                (config, Some(config_handler))
            }
            Err(_) => (Config::default(), None),
        };

        let has_credentials = config.has_credentials();
        let gym_uuid = if config.gym_uuid.is_empty() {
            None
        } else {
            Some(config.gym_uuid.clone())
        };
        let gym_name = if config.gym_name.is_empty() {
            None
        } else {
            Some(config.gym_name.clone())
        };

        let plus_one_count = config.plus_ones.len();
        let plus_one_clients: Vec<reqwest::Client> = (0..plus_one_count)
            .map(|_| api::create_client())
            .collect();
        let plus_one_uuids: Vec<Option<String>> = vec![None; plus_one_count];

        let mut app = AppModel {
            core,
            context_page: ContextPage::default(),
            about,
            nav,
            key_binds: HashMap::new(),
            login_username: config.username.clone(),
            login_pin: config.pin.clone(),
            config,
            config_handler,

            client,
            user_uuid: None,
            gym_uuid,
            gym_name,
            login_error: None,

            classes: Vec::new(),
            classes_error: None,
            busyness: None,
            check_ins: Vec::new(),
            history_error: None,
            loading: has_credentials,

            notified_classes: HashSet::new(),
            gym_warning: None,
            warning_window_ids: Vec::new(),

            editing_note_key: String::new(),
            editing_note_text: String::new(),

            plus_one_clients,
            plus_one_uuids,
            plus_one_booked_by: HashMap::new(),
            plus_one_form_name: String::new(),
            plus_one_form_username: String::new(),
            plus_one_form_pin: String::new(),
        };

        let login_task = if has_credentials {
            app.do_login()
        } else {
            Task::none()
        };
        let title_task = app.update_title();

        (app, Task::batch([login_task, title_task]))
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let mut menu_items = vec![
            menu::Item::Button(fl!("about"), None, MenuAction::About),
        ];

        if self.user_uuid.is_some() {
            menu_items.push(menu::Item::Divider);
            menu_items.push(menu::Item::Button("Plus Ones".to_string(), None, MenuAction::ManagePlusOnes));
            menu_items.push(menu::Item::Divider);
            menu_items.push(menu::Item::Button(fl!("logout"), None, MenuAction::Logout));
        }

        let menu_bar = menu::bar(vec![menu::Tree::with_children(
            menu::root(fl!("view")).apply(Element::from),
            menu::items(&self.key_binds, menu_items),
        )]);

        vec![menu_bar.into()]
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav)
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }

        let spacing = cosmic::theme::spacing();
        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::LaunchUrl(url.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
            ContextPage::PlusOnes => context_drawer::context_drawer(
                self.view_plus_ones(),
                Message::ToggleContextPage(ContextPage::PlusOnes),
            )
            .title("Plus Ones"),
            ContextPage::ClassNote => {
                let note_content = widget::column::with_capacity(3)
                    .spacing(spacing.space_m)
                    .push(widget::text::body(
                        "This note will appear on every future occurrence of this class.",
                    ))
                    .push(
                        widget::text_input("Add a note...", &self.editing_note_text)
                            .on_input(Message::ClassNoteChanged)
                            .id(note_input_id()),
                    )
                    .push(
                        widget::row::with_capacity(2)
                            .spacing(spacing.space_s)
                            .push(
                                widget::button::suggested("Save")
                                    .on_press(Message::SaveClassNote),
                            )
                            .push(
                                widget::button::text("Clear")
                                    .on_press(Message::ClassNoteChanged(String::new())),
                            ),
                    );
                context_drawer::context_drawer(
                    note_content,
                    Message::ToggleContextPage(ContextPage::ClassNote),
                )
                .title("Class Note")
            }
        })
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let space_m = cosmic::theme::spacing().space_m;

        let content: Element<_> = if !self.config.has_credentials() && self.user_uuid.is_none() {
            self.view_setup()
        } else if self.login_error.is_some() || self.user_uuid.is_none() {
            self.view_login_state()
        } else {
            match self.nav.active_data::<Page>() {
                Some(Page::Classes) => self.view_classes(),
                Some(Page::History) => self.view_history(),
                _ => self.view_classes(),
            }
        };

        // Wrap content with warning bar if needed
        let content_with_warning = if let Some(ref warning_text) = self.gym_warning {
            let warning_bar = widget::container(
                widget::text(warning_text)
                    .size(20)
                    .align_x(Horizontal::Center)
            )
            .width(Length::Fill)
            .height(80.0)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .class(cosmic::theme::Container::custom(|_theme| {
                widget::container::Style {
                    background: Some(cosmic::iced::Background::Color(
                        cosmic::iced::Color::from_rgb(0.8, 0.0, 0.0)
                    )),
                    text_color: Some(cosmic::iced::Color::WHITE),
                    ..Default::default()
                }
            }));

            widget::column::with_capacity(2)
                .push(warning_bar)
                .push(content)
                .into()
        } else {
            content
        };

        widget::container(content_with_warning)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(space_m)
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let config_sub = self
            .core()
            .watch_config::<Config>(Self::APP_ID)
            .map(|update| Message::UpdateConfig(update.config));

        let reminder_sub = cosmic::iced::time::every(std::time::Duration::from_secs(30))
            .map(|_| Message::CheckReminders);

        Subscription::batch([config_sub, reminder_sub])
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::LoggedIn(result) => match result {
                Ok(response) => {
                    self.user_uuid = Some(response.uuid);
                    self.login_error = None;

                    self.gym_uuid = Some(response.home_club_uuid.clone());
                    self.gym_name = Some(response.home_club_name.clone());

                    if let Some(ref config_handler) = self.config_handler {
                        let _ = self.config.set_username(config_handler, self.login_username.clone());
                        let _ = self.config.set_pin(config_handler, self.login_pin.clone());
                        let _ = self.config.set_gym_uuid(config_handler, response.home_club_uuid);
                        let _ = self.config.set_gym_name(config_handler, response.home_club_name);
                    }

                    return Task::batch([self.fetch_all_data(), self.do_plus_one_logins()]);
                }
                Err(e) => {
                    self.login_error = Some(e);
                    self.loading = false;
                }
            },

            Message::LoginUsernameChanged(value) => {
                self.login_username = value;
            }
            Message::LoginPinChanged(value) => {
                self.login_pin = value;
            }
            Message::SubmitLogin => {
                if self.login_username.is_empty() || self.login_pin.is_empty() {
                    self.login_error = Some("Please enter both email and PIN".to_string());
                    return Task::none();
                }
                self.loading = true;
                self.login_error = None;
                return self.do_login();
            }
            Message::Logout => {
                self.user_uuid = None;
                self.gym_uuid = None;
                self.gym_name = None;
                self.classes.clear();
                self.busyness = None;
                self.check_ins.clear();
                self.login_error = None;
                self.loading = false;
                self.login_username.clear();
                self.login_pin.clear();
                self.config.username.clear();
                self.config.pin.clear();
                self.config.gym_uuid.clear();
                self.config.gym_name.clear();

                if let Some(ref config_handler) = self.config_handler {
                    let _ = self.config.set_username(config_handler, String::new());
                    let _ = self.config.set_pin(config_handler, String::new());
                    let _ = self.config.set_gym_uuid(config_handler, String::new());
                    let _ = self.config.set_gym_name(config_handler, String::new());
                }
            }

            Message::ClassesLoaded(result) => {
                self.loading = false;
                match result {
                    Ok(mut classes) => {
                        classes.sort_by_key(|c| c.start_date_time.unwrap_or(i64::MAX));
                        self.classes = classes;
                        self.classes_error = None;
                        crate::calendar::sync_calendar(&self.classes, self.gym_name.as_deref());
                    }
                    Err(e) => {
                        self.classes_error = Some(e);
                    }
                }
            }

            Message::BusynessLoaded(result) => match result {
                Ok(b) => self.busyness = Some(b),
                Err(e) => eprintln!("Busyness error: {e}"),
            },

            Message::HistoryLoaded(result) => match result {
                Ok(mut check_ins) => {
                    // Sort by date descending (most recent first)
                    check_ins.sort_by(|a, b| {
                        b.check_in_date
                            .as_deref()
                            .unwrap_or("")
                            .cmp(a.check_in_date.as_deref().unwrap_or(""))
                    });
                    self.check_ins = check_ins;
                    self.history_error = None;
                }
                Err(e) => {
                    self.history_error = Some(e);
                }
            },

            Message::BookClass(class_id) => {
                let client = self.client.clone();
                let uuid = self.user_uuid.clone().unwrap_or_default();
                let gym_uuid = self.gym_uuid.clone().unwrap_or_default();
                let cid = class_id.clone();
                return Task::perform(
                    async move {
                        api::book_class(&client, &gym_uuid, &cid, &uuid)
                            .await
                            .map(|_| class_id)
                    },
                    |result| cosmic::Action::App(Message::BookingResult(result)),
                );
            }

            Message::BookingResult(result) => {
                match &result {
                    Ok(_) => {}
                    Err(e) => eprintln!("Booking error: {e}"),
                }
                return self.fetch_classes();
            }

            Message::CancelClass(class_id) => {
                let client = self.client.clone();
                let uuid = self.user_uuid.clone().unwrap_or_default();
                let gym_uuid = self.gym_uuid.clone().unwrap_or_default();
                let cid = class_id.clone();
                return Task::perform(
                    async move {
                        api::cancel_class(&client, &gym_uuid, &cid, &uuid)
                            .await
                            .map(|_| class_id)
                    },
                    |result| cosmic::Action::App(Message::CancelResult(result)),
                );
            }

            Message::CancelResult(result) => {
                match &result {
                    Ok(_) => {}
                    Err(e) => eprintln!("Cancel error: {e}"),
                }
                return self.fetch_classes();
            }

            Message::OpenClassNote(key) => {
                let note = self.config.class_notes.get(&key).cloned().unwrap_or_default();
                self.editing_note_key = key;
                self.editing_note_text = note;
                self.context_page = ContextPage::ClassNote;
                self.core.window.show_context = true;
                return widget::text_input::focus(note_input_id());
            }

            Message::ClassNoteChanged(text) => {
                self.editing_note_text = text;
            }

            Message::SaveClassNote => {
                let key = self.editing_note_key.clone();
                let mut new_notes = self.config.class_notes.clone();
                if self.editing_note_text.is_empty() {
                    new_notes.remove(&key);
                } else {
                    new_notes.insert(key, self.editing_note_text.clone());
                }
                if let Some(ref config_handler) = self.config_handler {
                    let _ = self.config.set_class_notes(config_handler, new_notes);
                }
                self.core.window.show_context = false;
            }

            Message::Refresh => {
                self.loading = true;
                self.plus_one_booked_by.clear();
                return Task::batch([self.fetch_all_data(), self.do_plus_one_logins()]);
            }

            Message::CheckReminders => {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let fifteen_min_ms: i64 = 15 * 60 * 1000;

                let mut found_warning = false;
                let mut warning_text = String::new();

                for class in &self.classes {
                    let is_booked = class.booked == Some(true);
                    let is_cancelled = class.cancelled == Some(true);
                    if !is_booked || is_cancelled {
                        continue;
                    }

                    let class_id = class.class_id().to_string();
                    if class_id.is_empty() {
                        continue;
                    }

                    if let Some(start_ms) = class.start_date_time {
                        let until_start = start_ms - now_ms;
                        // Show warning when class is 0–15 minutes away
                        if until_start > 0 && until_start <= fifteen_min_ms {
                            let name = class.display_name().to_string();
                            let time = class.formatted_time();
                            warning_text = format!("🏋️ {name} starts at {time} — Time for the gym!");
                            found_warning = true;

                            // Also show desktop notification once
                            if !self.notified_classes.contains(&class_id) {
                                if let Err(e) = notify_rust::Notification::new()
                                    .summary("Gym class in 15 minutes")
                                    .body(&format!("{name} starts at {time} — time to get ready!"))
                                    .icon("x-office-calendar")
                                    .urgency(notify_rust::Urgency::Critical)
                                    .show()
                                {
                                    eprintln!("Notification error: {e}");
                                }
                                self.notified_classes.insert(class_id);
                            }
                            break;
                        }
                    }
                }

                // Update warning state and windows
                if found_warning {
                    self.gym_warning = Some(warning_text.clone());

                    // Open warning windows if not already open (one per monitor)
                    if self.warning_window_ids.is_empty() {
                        return self.open_warning_windows();
                    }
                } else {
                    self.gym_warning = None;

                    // Close all warning windows if open
                    if !self.warning_window_ids.is_empty() {
                        let close_tasks: Vec<_> = self.warning_window_ids
                            .drain(..)
                            .map(cosmic::iced::window::close)
                            .collect();
                        return Task::batch(close_tasks);
                    }
                }
            }

            Message::WarningWindowOpened(id) => {
                self.warning_window_ids.push(id);
            }

            Message::WarningWindowClosed => {
                // Window was closed - remove it from tracking
                // Note: we don't know which window, so we'll let it be cleaned up on next check
            }

            Message::CloseRequested => {
                // If there's a gym warning, do nothing (prevent close)
                // Otherwise, allow the app to exit by closing the window
                if self.gym_warning.is_none() {
                    if let Some(window_id) = self.core.main_window_id() {
                        return cosmic::iced::window::close(window_id);
                    }
                }
                // If there's a warning, just ignore the close request
            }

            Message::PlusOneLoggedIn(index, result) => {
                match result {
                    Ok(response) => {
                        if index < self.plus_one_uuids.len() {
                            self.plus_one_uuids[index] = Some(response.uuid);
                            return self.fetch_plus_one_classes(index);
                        }
                    }
                    Err(e) => eprintln!("Plus One login error: {e}"),
                }
            }

            Message::PlusOneClassesLoaded(index, result) => {
                if let Ok(classes) = result {
                    if index < self.config.plus_ones.len() {
                        let name = self.config.plus_ones[index].name.clone();
                        for class in &classes {
                            if class.booked == Some(true) {
                                let class_id = class.class_id().to_string();
                                if !class_id.is_empty() {
                                    self.plus_one_booked_by
                                        .entry(class_id)
                                        .or_default()
                                        .push(name.clone());
                                }
                            }
                        }
                    }
                }
            }

            Message::AddPlusOne => {
                if self.plus_one_form_name.is_empty()
                    || self.plus_one_form_username.is_empty()
                    || self.plus_one_form_pin.is_empty()
                {
                    return Task::none();
                }
                let new_plus_one = PlusOneConfig {
                    name: std::mem::take(&mut self.plus_one_form_name),
                    username: std::mem::take(&mut self.plus_one_form_username),
                    pin: std::mem::take(&mut self.plus_one_form_pin),
                };
                let mut new_plus_ones = self.config.plus_ones.clone();
                new_plus_ones.push(new_plus_one);
                if let Some(ref config_handler) = self.config_handler {
                    let _ = self.config.set_plus_ones(config_handler, new_plus_ones);
                }
                self.plus_one_clients.push(api::create_client());
                self.plus_one_uuids.push(None);
                let index = self.plus_one_clients.len() - 1;
                return self.login_plus_one(index);
            }

            Message::RemovePlusOne(index) => {
                if index < self.config.plus_ones.len() {
                    let name = self.config.plus_ones[index].name.clone();
                    let mut new_plus_ones = self.config.plus_ones.clone();
                    new_plus_ones.remove(index);
                    if let Some(ref config_handler) = self.config_handler {
                        let _ = self.config.set_plus_ones(config_handler, new_plus_ones);
                    }
                    if index < self.plus_one_clients.len() {
                        self.plus_one_clients.remove(index);
                    }
                    if index < self.plus_one_uuids.len() {
                        self.plus_one_uuids.remove(index);
                    }
                    for names in self.plus_one_booked_by.values_mut() {
                        names.retain(|n| n != &name);
                    }
                    self.plus_one_booked_by.retain(|_, names| !names.is_empty());
                }
            }

            Message::PlusOneFormNameChanged(v) => self.plus_one_form_name = v,
            Message::PlusOneFormUsernameChanged(v) => self.plus_one_form_username = v,
            Message::PlusOneFormPinChanged(v) => self.plus_one_form_pin = v,

            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = context_page;
                    self.core.window.show_context = true;
                }
            }

            Message::UpdateConfig(config) => {
                self.config = config;
            }

            Message::LaunchUrl(url) => match open::that_detached(&url) {
                Ok(()) => {}
                Err(err) => {
                    eprintln!("failed to open {url:?}: {err}");
                }
            },
        }
        Task::none()
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<cosmic::Action<Self::Message>> {
        self.nav.activate(id);

        // Fetch history when switching to history tab
        let data_task = if self.nav.active_data::<Page>() == Some(&Page::History)
            && self.check_ins.is_empty()
        {
            self.fetch_history()
        } else {
            Task::none()
        };

        let title_task = self.update_title();
        Task::batch([data_task, title_task])
    }

    fn on_close_requested(&self, id: cosmic::iced::window::Id) -> Option<Self::Message> {
        // Check if this is a warning window
        if self.warning_window_ids.contains(&id) {
            return Some(Message::WarningWindowClosed);
        }

        // For main window, intercept close requests
        Some(Message::CloseRequested)
    }

    fn view_window(&self, id: cosmic::iced::window::Id) -> Element<Self::Message> {
        // Check if this is a warning window
        if self.warning_window_ids.contains(&id) {
            return self.view_warning_window();
        }

        // Otherwise, use the default view
        self.view()
    }
}

impl AppModel {
    fn open_warning_windows(&self) -> Task<cosmic::Action<Message>> {
        // Get actual monitor information from the system
        let monitors = get_monitor_info();

        let mut tasks = Vec::new();

        if monitors.is_empty() {
            // Fallback: create one window if we can't detect monitors
            eprintln!("Could not detect monitors, using fallback");
            let settings = cosmic::iced::window::Settings {
                size: cosmic::iced::Size::new(1920.0, 80.0),
                position: cosmic::iced::window::Position::Specific(cosmic::iced::Point::new(0.0, 0.0)),
                decorations: false,
                transparent: false,
                level: cosmic::iced::window::Level::AlwaysOnTop,
                ..Default::default()
            };

            let (id, task) = cosmic::iced::window::open(settings);
            tasks.push(task.map(move |_| cosmic::Action::App(Message::WarningWindowOpened(id))));
        } else {
            // Create one window per monitor
            for monitor in monitors {
                eprintln!("Creating warning window for monitor at {}x{} ({}x{})",
                    monitor.x, monitor.y, monitor.width, monitor.height);

                let settings = cosmic::iced::window::Settings {
                    size: cosmic::iced::Size::new(monitor.width as f32, 80.0),
                    position: cosmic::iced::window::Position::Specific(
                        cosmic::iced::Point::new(monitor.x as f32, monitor.y as f32)
                    ),
                    decorations: false,
                    transparent: false,
                    level: cosmic::iced::window::Level::AlwaysOnTop,
                    resizable: false,
                    exit_on_close_request: false,
                    ..Default::default()
                };

                let (id, task) = cosmic::iced::window::open(settings);
                tasks.push(task.map(move |_| cosmic::Action::App(Message::WarningWindowOpened(id))));
            }
        }

        Task::batch(tasks)
    }

    fn view_warning_window(&self) -> Element<Message> {
        let warning_text = self.gym_warning.as_deref().unwrap_or("Time for the gym!");

        widget::container(
            widget::text(warning_text)
                .size(24)
                .align_x(Horizontal::Center)
                .width(Length::Fill)
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(cosmic::theme::Container::custom(|_theme| {
            widget::container::Style {
                background: Some(cosmic::iced::Background::Color(
                    cosmic::iced::Color::from_rgb(0.8, 0.0, 0.0)
                )),
                text_color: Some(cosmic::iced::Color::WHITE),
                ..Default::default()
            }
        }))
        .into()
    }

    pub fn update_title(&mut self) -> Task<cosmic::Action<Message>> {
        let mut window_title = fl!("app-title");

        if let Some(page) = self.nav.text(self.nav.active()) {
            window_title.push_str(" — ");
            window_title.push_str(page);
        }

        if let Some(id) = self.core.main_window_id() {
            self.set_window_title(window_title, id)
        } else {
            Task::none()
        }
    }

    fn do_login(&self) -> Task<cosmic::Action<Message>> {
        let client = self.client.clone();
        let username = self.login_username.clone();
        let pin = self.login_pin.clone();
        Task::perform(
            async move { api::login(&client, &username, &pin).await },
            |result| cosmic::Action::App(Message::LoggedIn(result)),
        )
    }

    fn fetch_all_data(&self) -> Task<cosmic::Action<Message>> {
        Task::batch([self.fetch_classes(), self.fetch_busyness(), self.fetch_history()])
    }

    fn fetch_classes(&self) -> Task<cosmic::Action<Message>> {
        let client = self.client.clone();
        let uuid = self.user_uuid.clone().unwrap_or_default();
        let gym_uuid = self.gym_uuid.clone().unwrap_or_default();
        Task::perform(
            async move { api::get_classes(&client, &gym_uuid, &uuid).await },
            |result| cosmic::Action::App(Message::ClassesLoaded(result)),
        )
    }

    fn fetch_busyness(&self) -> Task<cosmic::Action<Message>> {
        let client = self.client.clone();
        let uuid = self.user_uuid.clone().unwrap_or_default();
        let gym_uuid = self.gym_uuid.clone().unwrap_or_default();
        Task::perform(
            async move { api::get_busyness(&client, &uuid, &gym_uuid).await },
            |result| cosmic::Action::App(Message::BusynessLoaded(result)),
        )
    }

    fn fetch_history(&self) -> Task<cosmic::Action<Message>> {
        let client = self.client.clone();
        let uuid = self.user_uuid.clone().unwrap_or_default();
        Task::perform(
            async move { api::get_check_in_history(&client, &uuid).await },
            |result| cosmic::Action::App(Message::HistoryLoaded(result)),
        )
    }

    fn login_plus_one(&self, index: usize) -> Task<cosmic::Action<Message>> {
        if index >= self.config.plus_ones.len() {
            return Task::none();
        }
        let client = self.plus_one_clients[index].clone();
        let username = self.config.plus_ones[index].username.clone();
        let pin = self.config.plus_ones[index].pin.clone();
        Task::perform(
            async move { api::login(&client, &username, &pin).await },
            move |result| cosmic::Action::App(Message::PlusOneLoggedIn(index, result)),
        )
    }

    fn do_plus_one_logins(&self) -> Task<cosmic::Action<Message>> {
        let tasks: Vec<_> = (0..self.config.plus_ones.len())
            .map(|i| self.login_plus_one(i))
            .collect();
        Task::batch(tasks)
    }

    fn fetch_plus_one_classes(&self, index: usize) -> Task<cosmic::Action<Message>> {
        let client = self.plus_one_clients[index].clone();
        let gym_uuid = self.gym_uuid.clone().unwrap_or_default();
        let uuid = match self.plus_one_uuids.get(index).and_then(|u| u.as_ref()) {
            Some(u) => u.clone(),
            None => return Task::none(),
        };
        Task::perform(
            async move { api::get_classes(&client, &gym_uuid, &uuid).await },
            move |result| cosmic::Action::App(Message::PlusOneClassesLoaded(index, result)),
        )
    }

    // --- Views ---

    fn view_setup(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();

        let mut col = widget::column::with_capacity(8)
            .spacing(spacing.space_m)
            .max_width(400.0)
            .align_x(Horizontal::Center);

        col = col.push(widget::text::title2(fl!("app-title")));
        col = col.push(widget::text("Sign in with your Gym Group credentials"));

        let email_input = widget::text_input("Email address", &self.login_username)
            .on_input(Message::LoginUsernameChanged);

        col = col.push(email_input);

        let pin_input = widget::secure_input("PIN", &self.login_pin, None::<Message>, true)
            .on_input(Message::LoginPinChanged);

        col = col.push(pin_input);

        if let Some(ref err) = self.login_error {
            col = col.push(widget::text(err.as_str()));
        }

        if self.loading {
            col = col.push(widget::text("Logging in..."));
        } else {
            let can_submit = !self.login_username.is_empty() && !self.login_pin.is_empty();
            let mut login_btn = widget::button::suggested("Log In");
            if can_submit {
                login_btn = login_btn.on_press(Message::SubmitLogin);
            }
            col = col.push(login_btn);
        }

        widget::container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
    }

    fn view_login_state(&self) -> Element<'_, Message> {
        let space_m = cosmic::theme::spacing().space_m;
        let mut col = widget::column::with_capacity(5).spacing(space_m);

        if self.loading && self.login_error.is_none() {
            col = col
                .push(widget::text::title3("Logging in to The Gym Group..."))
                .push(widget::text("Connecting to your account"))
                .align_x(Horizontal::Center);
        } else if let Some(ref err) = self.login_error {
            col = col
                .push(widget::text::title3("Login Failed"))
                .push(widget::text(err.as_str()))
                .push(
                    widget::row::with_capacity(2)
                        .spacing(space_m)
                        .push(
                            widget::button::suggested("Retry")
                                .on_press(Message::SubmitLogin),
                        )
                        .push(
                            widget::button::destructive("Change Account")
                                .on_press(Message::Logout),
                        ),
                )
                .align_x(Horizontal::Center);
        }

        widget::container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
    }

    fn view_classes(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut content = widget::column::with_capacity(10).spacing(spacing.space_m);

        // Header with refresh button
        let header = widget::row::with_capacity(2)
            .push(widget::text::title3("Upcoming Classes"))
            .push(widget::horizontal_space())
            .push(
                widget::button::text("Refresh")
                    .on_press(Message::Refresh),
            )
            .align_y(Alignment::Center)
            .spacing(spacing.space_s);
        content = content.push(header);

        // Busyness card
        if let Some(ref busy) = self.busyness {
            let gym_name = busy
                .gym_location_name
                .as_deref()
                .or(self.gym_name.as_deref())
                .unwrap_or("Unknown Gym");
            let capacity = busy.current_capacity.unwrap_or(0);
            let percentage = busy.current_percentage.unwrap_or(0.0);

            let busyness_section = cosmic::widget::settings::section()
                .title(gym_name)
                .add(cosmic::widget::settings::item(
                    format!("{capacity} people in gym ({percentage:.0}% full)"),
                    widget::text(""),
                ));
            content = content.push(busyness_section);
        }

        // Loading state
        if self.loading {
            content = content.push(widget::text("Loading classes..."));
        } else if let Some(ref err) = self.classes_error {
            content = content.push(widget::text(format!("Error: {err}")));
        } else if self.classes.is_empty() {
            content = content.push(widget::text("No upcoming classes found"));
        } else {
            // Group classes by day
            let mut current_day = String::new();
            let new_section = || {
                cosmic::widget::settings::section::with_column(
                    cosmic::widget::list_column()
                        .list_item_padding([spacing.space_xxs, 0])
                        .divider_padding(0),
                )
            };
            let mut section = new_section();
            let mut has_items = false;

            for class in &self.classes {
                let day = class
                    .start_date_time
                    .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
                    .map(|dt| dt.with_timezone(&chrono::Local).format("%A %d %b").to_string())
                    .unwrap_or_default();

                if day != current_day {
                    if has_items {
                        content = content.push(section);
                    }
                    section = new_section().title(day.clone());
                    current_day = day;
                    has_items = false;
                }

                let name = class.display_name().to_string();
                let time = class.formatted_time();
                let instructor = class.instructor_name();
                let spots = class.spots_text();
                let duration = class
                    .duration_minutes()
                    .map(|m| format!(" ({m}min)"))
                    .unwrap_or_default();

                let label = if instructor.is_empty() {
                    format!("{time}  {name}{duration}")
                } else {
                    format!("{time}  {name} — {instructor}{duration}")
                };

                let is_booked = class.booked.unwrap_or(false);
                let is_cancelled = class.cancelled.unwrap_or(false);
                let class_id = class.class_id().to_string();

                let action_button: Element<'_, Message> = {
                    let btn: Element<'_, Message> = if is_cancelled {
                        widget::text("Cancelled").into()
                    } else if is_booked {
                        widget::button::destructive("Cancel")
                            .on_press(Message::CancelClass(class_id))
                            .into()
                    } else if class.is_full() {
                        widget::text("Full").into()
                    } else {
                        widget::button::suggested("Book")
                            .on_press(Message::BookClass(class_id))
                            .into()
                    };
                    widget::container(btn)
                        .padding(cosmic::iced::Padding {
                            right: spacing.space_m as f32,
                            ..Default::default()
                        })
                        .into()
                };

                let item_label = if spots.is_empty() {
                    label
                } else {
                    format!("{label}  ({spots})")
                };

                let note_key = class_note_key(class);
                let note_text = self.config.class_notes.get(&note_key).cloned().unwrap_or_default();

                let note_button = widget::button::text("✎")
                    .on_press(Message::OpenClassNote(note_key));

                let plus_one_text = self
                    .plus_one_booked_by
                    .get(class.class_id())
                    .map(|names| format!("Also booked: {}", names.join(", ")))
                    .unwrap_or_default();

                let description = match (note_text.is_empty(), plus_one_text.is_empty()) {
                    (false, false) => format!("{note_text}\n{plus_one_text}"),
                    (false, true) => note_text,
                    (true, false) => plus_one_text,
                    (true, true) => String::new(),
                };

                let settings_item = if description.is_empty() {
                    cosmic::widget::settings::item::builder(item_label)
                        .icon(note_button)
                        .control(action_button)
                } else {
                    cosmic::widget::settings::item::builder(item_label)
                        .icon(note_button)
                        .description(description)
                        .control(action_button)
                };

                section = section.add(settings_item);
                has_items = true;
            }

            if has_items {
                content = content.push(section);
            }
        }

        widget::scrollable(
            widget::container(content)
                .padding(cosmic::iced::Padding {
                    right: spacing.space_m as f32,
                    ..Default::default()
                }),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn view_plus_ones(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut col = widget::column::with_capacity(12).spacing(spacing.space_m);

        col = col.push(widget::text::body(
            "Add family members or friends so you can see when they're also booked into the same classes.",
        ));

        for (i, plus_one) in self.config.plus_ones.iter().enumerate() {
            let status = if self.plus_one_uuids.get(i).and_then(|u| u.as_ref()).is_some() {
                "Connected"
            } else {
                "Connecting..."
            };
            let row = widget::row::with_capacity(3)
                .spacing(spacing.space_s)
                .align_y(Alignment::Center)
                .push(widget::text(&plus_one.name).width(Length::Fill))
                .push(widget::text(status))
                .push(
                    widget::button::destructive("Remove")
                        .on_press(Message::RemovePlusOne(i)),
                );
            col = col.push(row);
        }

        if !self.config.plus_ones.is_empty() {
            col = col.push(widget::divider::horizontal::default());
        }

        col = col.push(widget::text::title4("Add a Plus One"));
        col = col.push(
            widget::text_input("Name (e.g. Alice)", &self.plus_one_form_name)
                .on_input(Message::PlusOneFormNameChanged),
        );
        col = col.push(
            widget::text_input("Email address", &self.plus_one_form_username)
                .on_input(Message::PlusOneFormUsernameChanged),
        );
        col = col.push(
            widget::secure_input("PIN", &self.plus_one_form_pin, None::<Message>, true)
                .on_input(Message::PlusOneFormPinChanged),
        );

        let can_add = !self.plus_one_form_name.is_empty()
            && !self.plus_one_form_username.is_empty()
            && !self.plus_one_form_pin.is_empty();
        let mut add_btn = widget::button::suggested("Add");
        if can_add {
            add_btn = add_btn.on_press(Message::AddPlusOne);
        }
        col = col.push(add_btn);

        col.into()
    }

    fn view_contribution_graph(&self) -> Element<'_, Message> {
        use chrono::Datelike;

        let today = chrono::Local::now().date_naive();
        let one_year_ago = today - chrono::TimeDelta::days(364);

        // Count check-ins per day
        let mut day_counts: HashMap<chrono::NaiveDate, u32> = HashMap::new();
        for check_in in &self.check_ins {
            if let Some(ref date_str) = check_in.check_in_date {
                if let Ok(dt) =
                    chrono::NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S")
                {
                    let date = dt.date();
                    if date >= one_year_ago && date <= today {
                        *day_counts.entry(date).or_insert(0) += 1;
                    }
                }
            }
        }

        let cell_size: f32 = 10.0;
        let cell_gap: u16 = 2;
        let cell_pitch = cell_size + cell_gap as f32;

        // Align to Monday-based weeks
        let start_offset = one_year_ago.weekday().num_days_from_monday() as i64;
        let grid_start = one_year_ago - chrono::TimeDelta::days(start_offset);
        let total_days = (today - grid_start).num_days() + 1;
        let total_weeks = ((total_days + 6) / 7) as usize;

        // Month label spans
        let mut month_spans: Vec<(u32, usize)> = Vec::new();
        let mut prev_month = 0u32;
        for week in 0..total_weeks {
            let monday = grid_start + chrono::TimeDelta::days((week as i64) * 7);
            let m = monday.month();
            if m != prev_month {
                month_spans.push((m, 1));
                prev_month = m;
            } else if let Some(last) = month_spans.last_mut() {
                last.1 += 1;
            }
        }

        // Month labels row
        let mut month_row = widget::row::with_capacity(month_spans.len());
        for &(month_num, weeks) in &month_spans {
            let width = (weeks as f32) * cell_pitch;
            let name = month_abbr(month_num);
            month_row = month_row.push(
                widget::container(widget::text::caption(name)).width(width),
            );
        }

        // Grid: row of week-columns
        let mut weeks_row = widget::row::with_capacity(total_weeks).spacing(cell_gap);
        for week in 0..total_weeks {
            let mut week_col = widget::column::with_capacity(7).spacing(cell_gap);
            for day in 0..7i64 {
                let date =
                    grid_start + chrono::TimeDelta::days((week as i64) * 7 + day);
                let in_range = date >= one_year_ago && date <= today;
                let count = if in_range {
                    day_counts.get(&date).copied().unwrap_or(0)
                } else {
                    0
                };
                let color = if !in_range {
                    cosmic::iced::Color::TRANSPARENT
                } else {
                    contribution_color(count)
                };
                let cell = widget::container(widget::text(""))
                    .width(cell_size)
                    .height(cell_size)
                    .class(cosmic::theme::Container::custom(move |_theme| {
                        widget::container::Style {
                            background: Some(cosmic::iced::Background::Color(color)),
                            border: cosmic::iced::Border {
                                radius: 2.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }));
                week_col = week_col.push(cell);
            }
            weeks_row = weeks_row.push(week_col);
        }

        let total_visits = day_counts.values().sum::<u32>();
        let mut col = widget::column::with_capacity(4).spacing(4);
        col = col.push(widget::text(format!(
            "{total_visits} gym visits in the last year"
        )));
        col = col.push(month_row);
        col = col.push(weeks_row);

        col.into()
    }

    fn view_history(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut content = widget::column::with_capacity(10).spacing(spacing.space_m);

        let header = widget::row::with_capacity(2)
            .push(widget::text::title3("Check-in History"))
            .push(widget::horizontal_space())
            .push(
                widget::button::text("Refresh")
                    .on_press(Message::Refresh),
            )
            .align_y(Alignment::Center)
            .spacing(spacing.space_s);
        content = content.push(header);

        if let Some(ref err) = self.history_error {
            content = content.push(widget::text(format!("Error: {err}")));
        } else if self.check_ins.is_empty() {
            content = content.push(widget::text("No check-in history found"));
        } else {
            content = content.push(self.view_contribution_graph());

            let mut section = cosmic::widget::settings::section().title("Recent Visits");

            for check_in in self.check_ins.iter().take(50) {
                let date = check_in.formatted_date();
                let location = check_in
                    .gym_location_name
                    .as_deref()
                    .unwrap_or("Unknown gym");
                let duration = check_in
                    .duration_minutes()
                    .map(|m| format!("{m} min"))
                    .unwrap_or_default();

                let label = format!("{date} — {location}");

                section = section.add(cosmic::widget::settings::item(
                    label,
                    widget::text(duration),
                ));
            }

            content = content.push(section);
        }

        widget::scrollable(
            widget::container(content)
                .padding(cosmic::iced::Padding {
                    right: spacing.space_m as f32,
                    ..Default::default()
                }),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

/// Stable key for a recurring class: "name_dayofweek_HH:MM".
/// Monday = 0, …, Sunday = 6 (ISO weekday ordering).
fn class_note_key(class: &crate::api::GymClass) -> String {
    let name = class.display_name().to_string();
    if let Some(ms) = class.start_date_time {
        if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
            use chrono::Datelike;
            let local = dt.with_timezone(&chrono::Local);
            let day = local.weekday().num_days_from_monday();
            let time = local.format("%H:%M").to_string();
            return format!("{name}_{day}_{time}");
        }
    }
    name
}

fn contribution_color(count: u32) -> cosmic::iced::Color {
    match count {
        0 => cosmic::iced::Color::from_rgba(0.5, 0.5, 0.5, 0.15),
        1 => cosmic::iced::Color::from_rgb(0.055, 0.267, 0.161),
        2 => cosmic::iced::Color::from_rgb(0.0, 0.427, 0.196),
        3 => cosmic::iced::Color::from_rgb(0.149, 0.651, 0.255),
        _ => cosmic::iced::Color::from_rgb(0.224, 0.827, 0.325),
    }
}

fn month_abbr(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "",
    }
}

/// The page to display in the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    Classes,
    History,
}

/// The context page to display in the context drawer.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ContextPage {
    #[default]
    About,
    ClassNote,
    PlusOnes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
    Logout,
    ManagePlusOnes,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::About => Message::ToggleContextPage(ContextPage::About),
            MenuAction::Logout => Message::Logout,
            MenuAction::ManagePlusOnes => Message::ToggleContextPage(ContextPage::PlusOnes),
        }
    }
}

fn get_monitor_info() -> Vec<MonitorInfo> {
    use wayland_client::{Connection, Dispatch, QueueHandle};
    use wayland_client::protocol::{wl_output, wl_registry};

    struct State {
        monitors: Vec<MonitorInfo>,
        pending_monitors: HashMap<u32, MonitorInfo>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for State {
        fn event(
            state: &mut Self,
            _registry: &wl_registry::WlRegistry,
            _event: wl_registry::Event,
            _data: &(),
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
            // Registry events handled elsewhere
        }
    }

    impl Dispatch<wl_output::WlOutput, u32> for State {
        fn event(
            state: &mut Self,
            _output: &wl_output::WlOutput,
            event: wl_output::Event,
            id: &u32,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
            let monitor = state.pending_monitors.entry(*id).or_insert(MonitorInfo {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            });

            match event {
                wl_output::Event::Geometry { x, y, .. } => {
                    monitor.x = x;
                    monitor.y = y;
                }
                wl_output::Event::Mode { width, height, .. } => {
                    monitor.width = width as u32;
                    monitor.height = height as u32;
                }
                wl_output::Event::Done => {
                    state.monitors.push(monitor.clone());
                }
                _ => {}
            }
        }
    }

    // Try to connect to Wayland
    let conn = match Connection::connect_to_env() {
        Ok(conn) => conn,
        Err(e) => {
            eprintln!("Failed to connect to Wayland: {}", e);
            return Vec::new();
        }
    };

    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let _registry = display.get_registry(&qh, ());

    let mut state = State {
        monitors: Vec::new(),
        pending_monitors: HashMap::new(),
    };

    // Try to get monitor info with a timeout
    for _ in 0..10 {
        if event_queue.roundtrip(&mut state).is_err() {
            break;
        }
        if !state.monitors.is_empty() {
            break;
        }
    }

    state.monitors
}
