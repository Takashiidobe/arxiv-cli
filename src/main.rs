use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::prelude::*;
use std::{collections::HashSet, fs::File};
use std::{error::Error, io};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Text},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame, Terminal,
};

use serde::Deserialize;
use serde::Serialize;

pub type Responses = Vec<Response>;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub authors: Vec<Vec<String>>,
    pub links: Vec<Link>,
    pub published: String,
    pub updated: String,
    pub categories: Vec<Category>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub href: String,
    pub rel: String,
    #[serde(rename = "type")]
    pub type_field: Option<String>,
    pub title: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub term: String,
    pub scheme: String,
}

const BASE_URL: &str = "https://arxiv-json-api.fly.dev";
const FILE_PATH: &str = ".arxiv-cli";

fn open_url(url: &str) {
    use std::process::Command;

    Command::new("xdg-open")
        .arg(url)
        .output()
        .expect("failed to execute process");
}

#[derive(Clone, Debug)]
struct Params {
    page: u16,
    query: String,
}

impl Params {
    pub fn new() -> Self {
        Self {
            page: 1,
            query: "algorithms".to_string(),
        }
    }

    pub fn next_page_by(&mut self, amount: u16) {
        let page = self.page;
        self.page = if page + amount < 1000 {
            page + amount
        } else {
            1000
        }
    }

    pub fn prev_page_by(&mut self, amount: u16) {
        let page = self.page;
        self.page = if page <= amount { 0 } else { page - amount }
    }

    pub fn set_query<S: Into<String> + std::fmt::Display>(&mut self, query: S) {
        self.query = query.to_string();
    }
}

#[derive(Clone)]
struct App {
    state: TableState,
    items: Responses,
    current: Option<usize>,
    ids: HashSet<String>,
}

fn get_ids() -> HashSet<String> {
    let home_dir = dirs::home_dir();
    if let Some(home) = home_dir {
        if let Ok(id) = std::fs::read_to_string(&format!("{}/{}", home.display(), FILE_PATH)) {
            let mut ids = HashSet::default();
            for url in id.lines() {
                ids.insert(url.to_string());
            }
            ids
        } else {
            HashSet::default()
        }
    } else {
        HashSet::default()
    }
}

impl App {
    fn new() -> App {
        App {
            state: TableState::default(),
            items: vec![],
            current: None,
            ids: HashSet::new(),
        }
    }

    pub fn save_ids(&self) -> std::io::Result<()> {
        let mut s = String::from("");
        let home_dir = dirs::home_dir();
        if let Some(home) = home_dir {
            let mut nyaa_file = File::options()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&format!("{}/{}", home.display(), FILE_PATH))?;
            for id in self.ids.iter() {
                s.push_str(&format!("{}\n", id));
            }

            write!(nyaa_file, "{}", s)?;
        };
        Ok(())
    }

    pub fn set_ids(&mut self, ids: HashSet<String>) {
        self.ids = ids;
    }

    pub fn add_id(&mut self, id: String) {
        self.ids.insert(id);
    }

    pub fn remove_id(&mut self, id: String) {
        self.ids.remove(&id);
    }

    pub fn update_items(&mut self, items: Responses) {
        self.items = items;
    }

    pub fn first_item(&mut self) {
        self.current = Some(0);
        self.state.select(Some(0))
    }

    pub fn last_item(&mut self) {
        let last = if self.items.is_empty() {
            Some(0)
        } else {
            Some(self.items.len() - 1)
        };
        self.current = last;
        self.state.select(last);
    }

    pub fn next_by(&mut self, amount: usize) {
        let i = match self.state.selected() {
            Some(i) => {
                if i + amount >= self.items.len() - 1 {
                    self.items.len() - 1
                } else {
                    i + amount
                }
            }
            None => 0,
        };
        self.current = Some(i);
        self.state.select(Some(i));
    }

    pub fn previous_by(&mut self, amount: usize) {
        let i = match self.state.selected() {
            Some(i) => match i {
                0 => 0,
                i => {
                    if amount >= i {
                        0
                    } else {
                        i - amount
                    }
                }
            },
            None => 0,
        };
        self.current = Some(i);
        self.state.select(Some(i));
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut app = App::new();
    app.set_ids(get_ids());
    let mut params = Params::new();
    let items = get_items(&params).await?;
    app.update_items(items);

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    run_app(&mut terminal, app, &mut params).await?;

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

// fetch the request
async fn get_items(params: &Params) -> Result<Responses, Box<dyn Error>> {
    let client = reqwest::Client::new();

    let Params { query, page } = params;

    let query = client
        .get(BASE_URL)
        .query(&[("q", &query.to_string()), ("p", &page.to_string())]);
    let res = query.send().await?.json::<Responses>().await?;

    Ok(res)
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
    params: &mut Params,
) -> Result<(), Box<dyn Error>> {
    let mut amount = String::from("");
    loop {
        terminal.draw(|f| ui(f, &mut app))?;
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('9') => amount.push('9'),
                KeyCode::Char('8') => amount.push('8'),
                KeyCode::Char('7') => amount.push('7'),
                KeyCode::Char('6') => amount.push('6'),
                KeyCode::Char('5') => amount.push('5'),
                KeyCode::Char('4') => amount.push('4'),
                KeyCode::Char('3') => amount.push('3'),
                KeyCode::Char('2') => amount.push('2'),
                KeyCode::Char('1') => amount.push('1'),
                KeyCode::Char('0') => amount.push('0'),
                KeyCode::Char('q') => {
                    app.save_ids()?;
                    return Ok(());
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.next_by(amount.parse::<usize>().unwrap_or(1));
                    amount = String::default();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.previous_by(amount.parse::<usize>().unwrap_or(1));
                    amount = String::default();
                }
                KeyCode::Char('G') => app.last_item(),
                KeyCode::Char('g') => app.first_item(),
                KeyCode::Char('n') => {
                    params.next_page_by(amount.parse::<u16>().unwrap_or(1));
                    let items = get_items(params).await?;
                    app.update_items(items);
                    terminal.draw(|f| ui(f, &mut app))?;
                }
                KeyCode::Char('p') => {
                    params.prev_page_by(amount.parse::<u16>().unwrap_or(1));
                    let items = get_items(params).await?;
                    app.update_items(items);
                    terminal.draw(|f| ui(f, &mut app))?;
                }
                KeyCode::Char('/') => {
                    let mut query = String::from("");
                    loop {
                        if let Event::Key(key) = event::read()? {
                            match key.code {
                                KeyCode::Char(c) => query.push(c),
                                KeyCode::Enter => break,
                                KeyCode::Backspace => {
                                    query.pop();
                                }
                                _ => {}
                            }
                        }
                        terminal.draw(|f| search_ui(f, &query))?;
                    }
                    params.set_query(query);
                    let items = get_items(params).await?;
                    app.update_items(items);
                    terminal.draw(|f| ui(f, &mut app))?;
                }
                KeyCode::Char('o') => {
                    let pdf_links = app.items[app.current.unwrap_or(0)]
                        .links
                        .iter()
                        .find(|link| link.title == Some("pdf".to_string()));

                    if let Some(link) = pdf_links {
                        open_url(&link.href);
                    }
                }
                KeyCode::Char('t') => {
                    let alternate_link = app.items[app.current.unwrap_or(0)]
                        .links
                        .iter()
                        .find(|link| link.rel == *"alternate");

                    if let Some(link) = alternate_link {
                        let html_link = link.href.replace("arxiv", "ar5iv");
                        open_url(&html_link);
                    }
                }
                KeyCode::Char('b') => {
                    params.set_query("");
                    let items = get_items(params).await?;
                    app.update_items(items);
                    terminal.draw(|f| ui(f, &mut app))?;
                }
                KeyCode::Char('h') => loop {
                    terminal.draw(|f| popup_ui(f))?;
                    if let Event::Key(_) = event::read()? {
                        break;
                    }
                },
                KeyCode::Char('s') => {
                    let id = &app.items[app.current.unwrap_or(0)].id;
                    app.add_id(id.to_string());
                }
                KeyCode::Char('d') => {
                    let id = &app.items[app.current.unwrap_or(0)].id;
                    app.remove_id(id.to_string());
                }
                _ => {}
            }
        }
    }
}

fn search_ui<B: Backend>(f: &mut Frame<B>, text: &str) {
    let size = f.size();

    let chunks = Layout::default()
        .constraints([Constraint::Percentage(20)].as_ref())
        .split(size);

    let paragraph = Paragraph::new(Span::styled(text, Style::default()))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, chunks[0]);
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let rects = Layout::default()
        .constraints([Constraint::Percentage(100)].as_ref())
        .margin(1)
        .split(f.size());

    let selected_style = Style::default().add_modifier(Modifier::REVERSED);
    let normal_style = Style::default().bg(Color::Blue);
    let header_cells = ["Seen", "Title", "Summary", "Authors", "Date"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Red)));
    let header = Row::new(header_cells)
        .style(normal_style)
        .height(1)
        .bottom_margin(1);
    let rows = app.items.iter().map(|item| {
        let Response {
            id,
            updated,
            title,
            summary,
            authors,
            ..
        } = item;
        let flattened_authors: Vec<_> = authors.iter().flatten().map(|x| x.to_string()).collect();
        let authors_str = flattened_authors.join(", ");
        let height = 8;

        let viewed = if app.ids.contains(id) { "✅" } else { "❌" };
        let cells = [viewed, title, summary, &authors_str, updated]
            .map(|x| Cell::from(Text::from(x.to_string())));
        Row::new(cells).height(height as u16).bottom_margin(1)
    });
    let t = Table::new(rows)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Table"))
        .highlight_style(selected_style)
        .highlight_symbol(">> ")
        .widths(&[
            Constraint::Percentage(2),
            Constraint::Percentage(32),
            Constraint::Percentage(38),
            Constraint::Percentage(16),
            Constraint::Percentage(6),
        ]);
    f.render_stateful_widget(t, rects[0], &mut app.state);
}

fn popup_ui<B: Backend>(f: &mut Frame<B>) {
    let size = f.size();

    const HELP_TEXT: &str = "
/ to search
s to mark the current spot as viewed until
<number> n to go to <number> pages next (like 5n to go 5 more pages)
<number> p to go to <number> pages previous (like 5p to go 5 fewer pages)
<number> j or down arrow to go down one item.
<number> k or up arrow to up one item.
o to open the selected item in the web browser.
t to open up the selected item's HTML version (if it has one).
";
    let paragraph = Paragraph::new(Span::from(HELP_TEXT))
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, size);
}
