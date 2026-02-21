// SPDX-License-Identifier: MPL-2.0

use crate::api::{self, Busyness, CheckIn, GymClass};
use crate::config::Config;
use crate::fl;
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use cosmic::{prelude::*, Task};
use std::collections::{HashMap, HashSet};

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

    // Reminders
    CheckReminders,

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

        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::LaunchUrl(url.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
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

        widget::container(content)
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

                    return self.fetch_all_data();
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

            Message::Refresh => {
                self.loading = true;
                return self.fetch_all_data();
            }

            Message::CheckReminders => {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let fifteen_min_ms: i64 = 15 * 60 * 1000;

                for class in &self.classes {
                    let is_booked = class.booked == Some(true);
                    let is_cancelled = class.cancelled == Some(true);
                    if !is_booked || is_cancelled {
                        continue;
                    }

                    let class_id = class.class_id().to_string();
                    if class_id.is_empty() || self.notified_classes.contains(&class_id) {
                        continue;
                    }

                    if let Some(start_ms) = class.start_date_time {
                        let until_start = start_ms - now_ms;
                        // Notify when class is 0–15 minutes away
                        if until_start > 0 && until_start <= fifteen_min_ms {
                            let name = class.display_name().to_string();
                            let time = class.formatted_time();
                            let body = format!("{name} starts at {time} — time to get ready!");

                            if let Err(e) = notify_rust::Notification::new()
                                .summary("Gym class in 15 minutes")
                                .body(&body)
                                .icon("x-office-calendar")
                                .urgency(notify_rust::Urgency::Critical)
                                .show()
                            {
                                eprintln!("Notification error: {e}");
                            }

                            self.notified_classes.insert(class_id);
                        }
                    }
                }
            }

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
}

impl AppModel {
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
            let mut section = cosmic::widget::settings::section();
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
                    section = cosmic::widget::settings::section().title(day.clone());
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

                let action_button: Element<'_, Message> = if is_cancelled {
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

                let item_label = if spots.is_empty() {
                    label
                } else {
                    format!("{label}  ({spots})")
                };

                section = section.add(cosmic::widget::settings::item(
                    item_label,
                    action_button,
                ));
                has_items = true;
            }

            if has_items {
                content = content.push(section);
            }
        }

        widget::scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
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

        let cell_size: f32 = 12.0;
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

        widget::scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
    Logout,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::About => Message::ToggleContextPage(ContextPage::About),
            MenuAction::Logout => Message::Logout,
        }
    }
}
