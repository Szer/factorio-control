use crate::{App, Error, Result, models::ServerState, now, storage};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::Read,
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct Research {
    pub force: String,
    pub name: String,
    pub progress: f64,
    pub level: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SampleState {
    Active,
    Paused,
    Collecting,
    Unavailable,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct UpsPoint {
    pub timestamp: u64,
    #[schema(required = true)]
    pub ups: Option<f64>,
    #[schema(required = true)]
    pub target_ups: Option<f64>,
    pub state: SampleState,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Started,
    Stopped,
    Joined,
    Left,
    ResearchFinished,
    ModAdded,
    ModUpdated,
    ModRemoved,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Activity {
    pub id: u64,
    pub timestamp: u64,
    pub kind: ActivityKind,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Snapshot {
    pub enabled: bool,
    pub available: bool,
    #[schema(required = true)]
    pub sampled_at: Option<u64>,
    #[schema(required = true)]
    pub players: Option<Vec<String>>,
    #[schema(required = true)]
    pub research: Option<Vec<Research>>,
    #[schema(required = true)]
    pub ups: Option<f64>,
    #[schema(required = true)]
    pub target_ups: Option<f64>,
    pub history: Vec<UpsPoint>,
    pub activity: Vec<Activity>,
    #[schema(required = true)]
    pub error: Option<String>,
}
#[derive(Clone)]
pub struct Session {
    address: SocketAddr,
    password: String,
    auto_pause: bool,
    research_watch: std::sync::Arc<std::sync::Mutex<Vec<Research>>>,
}
impl Session {
    pub fn new(auto_pause: bool) -> Result<Self> {
        let port = std::env::var("FC_RCON_PORT")
            .unwrap_or_else(|_| "27015".into())
            .parse::<std::num::NonZeroU16>()
            .map_err(|_| Error::bad("FC_RCON_PORT must be 1–65535"))?;
        let address = SocketAddr::from(([127, 0, 0, 1], port.get()));
        let mut random = [0_u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let password = random.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            address,
            password,
            auto_pause,
            research_watch: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }
    pub fn configure(&self, command: &mut tokio::process::Command) {
        command
            .arg("--rcon-bind")
            .arg(self.address.to_string())
            .arg("--rcon-password")
            .arg(&self.password);
    }
    pub fn redaction(&self) -> String {
        self.password.clone()
    }
}
#[derive(Default)]
pub struct Runtime {
    session: Option<Session>,
    previous: Option<(WireSample, Instant)>,
    completed: std::collections::BTreeMap<(String, String), Completed>,
    sampled_at: Option<u64>,
    players: Option<Vec<String>>,
    research: Option<Vec<Research>>,
    ups: Option<f64>,
    target: Option<f64>,
    history: VecDeque<UpsPoint>,
    activity: VecDeque<Activity>,
    next_activity: u64,
    error: Option<String>,
}
impl Runtime {
    pub fn begin(&mut self, session: Session) {
        self.session = Some(session);
        self.clear_sample();
        self.players = Some(Vec::new());
    }
    pub fn auto_paused(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.auto_pause)
            && self.players.as_ref().is_some_and(Vec::is_empty)
    }
    pub fn auto_pause_enabled(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.auto_pause)
    }
    pub fn end(&mut self) {
        self.session = None;
        self.clear_sample();
        self.players = Some(Vec::new());
    }
    fn clear_sample(&mut self) {
        if self.sampled_at.is_some() {
            self.history.push_back(UpsPoint {
                timestamp: now(),
                ups: None,
                target_ups: None,
                state: SampleState::Unavailable,
            });
            while self.history.len() > 86400 {
                self.history.pop_front();
            }
        }
        self.previous = None;
        self.completed.clear();
        self.sampled_at = None;
        self.research = None;
        self.ups = None;
        self.target = None;
        self.error = None;
    }
    pub fn activity(&mut self, kind: ActivityKind, message: String) {
        self.next_activity = self.next_activity.saturating_add(1);
        self.activity.push_back(Activity {
            id: self.next_activity,
            timestamp: now(),
            kind,
            message,
        });
        while self.activity.len() > 200 {
            self.activity.pop_front();
        }
    }
    pub fn observe_log(&mut self, text: &str) -> bool {
        let event = text
            .split_once("[JOIN] ")
            .map(|(prefix, name)| (prefix, true, name))
            .or_else(|| {
                text.split_once("[LEAVE] ")
                    .map(|(prefix, name)| (prefix, false, name))
            });
        let Some((prefix, joined, name)) = event else {
            return false;
        };
        if !prefix.chars().all(|c| {
            c.is_ascii_digit()
                || c.is_ascii_whitespace()
                || matches!(c, ':' | '.' | '-' | 'T' | 'Z')
        }) {
            return false;
        }
        let suffix = if joined {
            " joined the game"
        } else {
            " left the game"
        };
        let Some((name, _)) = name.rsplit_once(suffix) else {
            return false;
        };
        let name = name.trim();
        if name.is_empty() || name.len() > 512 {
            return false;
        }
        let players = self.players.get_or_insert_with(Vec::new);
        if joined {
            if !players.iter().any(|p| p == name) {
                players.push(name.into());
                players.sort();
            }
        } else {
            players.retain(|p| p != name);
        }
        self.activity(
            if joined {
                ActivityKind::Joined
            } else {
                ActivityKind::Left
            },
            format!("{name} {}", if joined { "joined" } else { "left" }),
        );
        true
    }
    pub fn snapshot(&self, enabled: bool, interval: u32) -> Snapshot {
        let fresh = self.sampled_at.is_some_and(|t| {
            now().saturating_sub(t) <= u64::from(interval).saturating_mul(3).max(10)
        });
        let available = enabled && fresh && self.error.is_none();
        Snapshot {
            enabled,
            available,
            sampled_at: self.sampled_at,
            players: self
                .players
                .clone()
                .or_else(|| self.session.is_none().then(Vec::new)),
            research: if available {
                self.research.clone()
            } else {
                None
            },
            ups: if available { self.ups } else { None },
            target_ups: if available { self.target } else { None },
            history: self.history.iter().cloned().collect(),
            activity: self.activity.iter().rev().cloned().collect(),
            error: self.error.clone(),
        }
    }
    fn accept(&mut self, sample: WireSample, observed: Instant, history_minutes: u32) {
        let ups = self
            .previous
            .as_ref()
            .and_then(|(last, time)| measured_ups(last, &sample, observed.duration_since(*time)));
        let finished = sample
            .completed
            .iter()
            .filter(|current| {
                self.completed
                    .get(&(current.force.clone(), current.name.clone()))
                    .is_some_and(|old| {
                        (!old.researched && current.researched) || current.level > old.level
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        if sample.full_scan {
            self.completed.clear();
        }
        for completed in &sample.completed {
            self.completed.insert(
                (completed.force.clone(), completed.name.clone()),
                completed.clone(),
            );
        }
        for completed in &finished {
            self.activity(
                ActivityKind::ResearchFinished,
                format!(
                    "{} finished researching {}",
                    completed.force, completed.name
                ),
            );
        }
        self.sampled_at = Some(now());
        self.players = Some(sample.players.clone());
        self.research = Some(sample.research.clone());
        self.ups = ups;
        self.target = Some(sample.speed * 60.0);
        self.error = None;
        self.history.push_back(UpsPoint {
            timestamp: now(),
            ups,
            target_ups: self.target,
            state: if sample.paused {
                SampleState::Paused
            } else if ups.is_some() {
                SampleState::Active
            } else {
                SampleState::Collecting
            },
        });
        let cutoff = now().saturating_sub(u64::from(history_minutes) * 60);
        while self.history.front().is_some_and(|p| p.timestamp < cutoff)
            || self.history.len() > 86400
        {
            self.history.pop_front();
        }
        self.previous = Some((sample, observed));
    }
    fn unavailable(&mut self, message: String) {
        self.previous = None;
        self.ups = None;
        self.error = Some(message);
        self.history.push_back(UpsPoint {
            timestamp: now(),
            ups: None,
            target_ups: None,
            state: SampleState::Unavailable,
        });
        while self.history.len() > 86400 {
            self.history.pop_front();
        }
    }
}
#[derive(Clone, Deserialize)]
struct Completed {
    force: String,
    name: String,
    level: u32,
    researched: bool,
}
#[derive(Clone, Deserialize)]
struct WireSample {
    tick: u64,
    speed: f64,
    paused: bool,
    #[serde(deserialize_with = "array_or_empty_table")]
    players: Vec<String>,
    #[serde(deserialize_with = "array_or_empty_table")]
    research: Vec<Research>,
    #[serde(deserialize_with = "array_or_empty_table")]
    completed: Vec<Completed>,
    #[serde(default)]
    full_scan: bool,
    #[serde(default, deserialize_with = "array_or_empty_table")]
    watch: Vec<Research>,
}
fn array_or_empty_table<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum LuaArray<T> {
        Array(Vec<T>),
        Table(std::collections::BTreeMap<String, serde::de::IgnoredAny>),
    }
    match LuaArray::<T>::deserialize(deserializer)? {
        LuaArray::Array(values) => Ok(values),
        LuaArray::Table(table) if table.is_empty() => Ok(Vec::new()),
        LuaArray::Table(_) => Err(serde::de::Error::custom(
            "expected array or empty Lua table",
        )),
    }
}
fn measured_ups(previous: &WireSample, current: &WireSample, elapsed: Duration) -> Option<f64> {
    if current.paused || previous.paused || elapsed.is_zero() {
        return None;
    }
    current
        .tick
        .checked_sub(previous.tick)
        .map(|ticks| ticks as f64 / elapsed.as_secs_f64())
}
fn lua_string(value: &str) -> String {
    let escaped = value
        .bytes()
        .map(|byte| format!("\\{byte:03}"))
        .collect::<String>();
    format!("\"{escaped}\"")
}
fn sample_command(watched: &[Research], reconcile: bool) -> Result<String> {
    if watched.len() > 1024
        || watched
            .iter()
            .any(|r| r.force.len() > 1024 || r.name.len() > 1024)
    {
        return Err(Error::bad("Too many research entries for telemetry"));
    }
    let mut by_force = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for research in watched {
        by_force
            .entry(&research.force)
            .or_default()
            .push(&research.name);
    }
    let watched = by_force
        .into_iter()
        .map(|(force, names)| {
            let names = names
                .into_iter()
                .map(|name| format!("[{}]=true", lua_string(name)))
                .collect::<Vec<_>>()
                .join(",");
            format!("[{}]={{{names}}}", lua_string(force))
        })
        .collect::<Vec<_>>()
        .join(",");
    let scan = if reconcile {
        "for name,t in pairs(f.technologies) do names[name]=true end; "
    } else {
        ""
    };
    let command = format!(
        "/sc local w={{{watched}}}; local p={{}}; for _,v in pairs(game.connected_players) do p[#p+1]=v.name end; local r={{}}; local c={{}}; local next={{}}; for _,f in pairs(game.forces) do local names=w[f.name] or {{}}; local active={{}}; if f.current_research then r[#r+1]={{force=f.name,name=f.current_research.name,progress=f.research_progress,level=f.current_research.level}}; active[f.current_research.name]=true end; for _,q in pairs(f.research_queue or {{}}) do active[q.name]=true end; for name,_ in pairs(active) do names[name]=true; local t=f.technologies[name]; if t then next[#next+1]={{force=f.name,name=t.name,progress=0,level=t.level}} end end; {scan}for name,_ in pairs(names) do local t=f.technologies[name]; if t then c[#c+1]={{force=f.name,name=t.name,level=t.level,researched=t.researched}} end end end; rcon.print(helpers.table_to_json({{tick=game.tick,speed=game.speed,paused=game.tick_paused,players=p,research=r,completed=c,watch=next,full_scan={reconcile}}}))"
    );
    if command.len() > 65526 {
        return Err(Error::bad("Research telemetry command exceeds RCON limit"));
    }
    Ok(command)
}
pub async fn run(app: App, generation: u64) {
    let mut last_reconciliation: Option<Instant> = None;
    loop {
        let settings = match storage::manager_settings(&app.manager_settings_path()) {
            Ok(document) => document.settings,
            Err(error) => {
                let mut inner = app.inner.lock().await;
                if inner.generation != generation || inner.child.is_none() {
                    return;
                }
                inner.telemetry.unavailable(format!(
                    "Unable to read telemetry settings: {}",
                    error.message
                ));
                drop(inner);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let request = {
            let mut inner = app.inner.lock().await;
            if inner.generation != generation || inner.child.is_none() {
                return;
            }
            let cutoff = now().saturating_sub(u64::from(settings.ups_history_minutes) * 60);
            while inner
                .telemetry
                .history
                .front()
                .is_some_and(|point| point.timestamp < cutoff)
            {
                inner.telemetry.history.pop_front();
            }
            if !settings.enable_script_telemetry {
                inner.telemetry.clear_sample();
                None
            } else if matches!(
                inner.status.state,
                ServerState::Running | ServerState::Paused
            ) {
                inner.telemetry.session.clone()
            } else {
                None
            }
        };
        if let Some(session) = request {
            let reconcile =
                last_reconciliation.is_none_or(|last| last.elapsed() >= Duration::from_secs(30));
            if reconcile {
                last_reconciliation = Some(Instant::now());
            }
            let (result, audit_failed) =
                resilient_sample(reconcile, |mode| timed_sample(&session, mode)).await;
            let mut inner = app.inner.lock().await;
            if inner.generation != generation || inner.child.is_none() {
                return;
            }
            if matches!(
                inner.status.state,
                ServerState::Running | ServerState::Paused
            ) {
                match result {
                    Ok((sample, observed)) => {
                        let state = if sample.paused {
                            ServerState::Paused
                        } else {
                            ServerState::Running
                        };
                        if inner.status.state != state {
                            inner.status.state = state;
                            inner.status.since = now();
                        }
                        inner
                            .telemetry
                            .accept(sample, observed, settings.ups_history_minutes);
                        if audit_failed {
                            inner.telemetry.error = Some("Full research reconciliation is unavailable; live samples still work".into());
                        }
                    }
                    Err(error) => inner.telemetry.unavailable(error.message),
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(u64::from(
            settings.poll_interval_seconds,
        )))
        .await;
    }
}
async fn timed_sample(session: &Session, reconcile: bool) -> Result<(WireSample, Instant)> {
    let started = Instant::now();
    let sample = tokio::time::timeout(Duration::from_secs(3), read_sample_mode(session, reconcile))
        .await
        .map_err(|_| Error::bad("Telemetry request timed out"))??;
    Ok((sample, started + started.elapsed() / 2))
}
async fn resilient_sample<T, F, Fut>(reconcile: bool, mut read: F) -> (Result<T>, bool)
where
    F: FnMut(bool) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let result = read(reconcile).await;
    if reconcile && result.is_err() {
        (read(false).await, true)
    } else {
        (result, false)
    }
}
async fn connect(session: &Session) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(session.address)
        .await
        .map_err(|_| Error::bad("Telemetry RCON connection unavailable"))?;
    send_packet(&mut stream, 1, 3, &session.password).await?;
    let mut authenticated = false;
    for _ in 0..4 {
        let (id, kind, _) = read_packet(&mut stream).await?;
        if id == -1 {
            return Err(Error::bad("Telemetry RCON authentication failed"));
        }
        if id == 1 && kind == 2 {
            authenticated = true;
            break;
        }
    }
    if !authenticated {
        return Err(Error::bad("Invalid telemetry authentication response"));
    }
    Ok(stream)
}
#[cfg(test)]
async fn read_sample(session: &Session) -> Result<WireSample> {
    read_sample_mode(session, false).await
}
async fn read_sample_mode(session: &Session, reconcile: bool) -> Result<WireSample> {
    let mut stream = connect(session).await?;
    let command = {
        let watched = session
            .research_watch
            .lock()
            .map_err(|_| Error::bad("Research tracking unavailable"))?;
        sample_command(&watched, reconcile)?
    };
    send_packet(&mut stream, 2, 2, &command).await?;
    let mut output = Vec::new();
    for _ in 0..256 {
        let (id, kind, body) = read_packet(&mut stream).await?;
        if id == 2 && kind == 0 {
            output.extend_from_slice(&body);
            if output.len() > 1024 * 1024 {
                return Err(Error::bad("Telemetry response exceeds size limit"));
            }
            let parsed = serde_json::from_slice::<WireSample>(&output);
            if let Err(error) = &parsed
                && error.is_data()
            {
                return Err(Error::bad(&format!("Invalid telemetry fields: {error}")));
            }
            if let Ok(mut sample) = parsed {
                if !sample.speed.is_finite()
                    || !(0.0..=10000.0).contains(&sample.speed)
                    || sample
                        .research
                        .iter()
                        .any(|r| !r.progress.is_finite() || !(0.0..=1.0).contains(&r.progress))
                {
                    return Err(Error::bad("Invalid telemetry values"));
                }
                // Empty-server auto-pause is separate from game.tick_paused.
                sample.paused |= session.auto_pause && sample.players.is_empty();
                if sample.watch.len() > 1024 {
                    return Err(Error::bad("Too many active research entries for telemetry"));
                }
                *session
                    .research_watch
                    .lock()
                    .map_err(|_| Error::bad("Research tracking unavailable"))? =
                    sample.watch.clone();
                return Ok(sample);
            }
            if output.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
                return Err(Error::bad(
                    "Factorio did not return telemetry; scripting may require confirmation",
                ));
            }
        }
    }
    Err(Error::bad("Invalid telemetry response"))
}
async fn send_packet(stream: &mut TcpStream, id: i32, kind: i32, body: &str) -> Result<()> {
    let length =
        i32::try_from(body.len() + 10).map_err(|_| Error::bad("RCON command too large"))?;
    let mut bytes = Vec::with_capacity(body.len() + 14);
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&id.to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(body.as_bytes());
    bytes.extend_from_slice(&[0, 0]);
    stream.write_all(&bytes).await?;
    Ok(())
}
async fn read_packet(stream: &mut TcpStream) -> Result<(i32, i32, Vec<u8>)> {
    let size = stream.read_i32_le().await?;
    if !(10..=65536).contains(&size) {
        return Err(Error::bad("Invalid RCON packet size"));
    }
    let id = stream.read_i32_le().await?;
    let kind = stream.read_i32_le().await?;
    let mut body = vec![0; usize::try_from(size - 8).map_err(|_| Error::bad("Invalid RCON size"))?];
    stream.read_exact(&mut body).await?;
    if !body.ends_with(&[0, 0]) {
        return Err(Error::bad("Invalid RCON terminator"));
    }
    body.truncate(body.len() - 2);
    Ok((id, kind, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(tick: u64, paused: bool) -> WireSample {
        WireSample {
            tick,
            speed: 1.0,
            paused,
            players: Vec::new(),
            research: Vec::new(),
            completed: Vec::new(),
            full_scan: false,
            watch: Vec::new(),
        }
    }
    #[test]
    fn research_query_is_bounded_and_escapes_untrusted_names() -> Result<()> {
        let watched = vec![Research {
            force: "a\";error('injected')".into(),
            name: "tech\\name".into(),
            progress: 0.5,
            level: 1,
        }];
        let command = sample_command(&watched, false)?;
        assert!(!command.contains("pairs(f.technologies)"));
        assert!(!command.contains("error('injected')"));
        assert!(sample_command(&vec![watched[0].clone(); 1025], false).is_err());
        assert_eq!(lua_string("a\""), "\"\\097\\034\"");
        Ok(())
    }
    #[test]
    fn measures_real_elapsed_time_and_never_confuses_pause_with_zero() {
        assert_eq!(
            measured_ups(
                &sample(0, false),
                &sample(90, false),
                Duration::from_secs(2)
            ),
            Some(45.0)
        );
        assert_eq!(
            measured_ups(
                &sample(90, false),
                &sample(90, true),
                Duration::from_secs(2)
            ),
            None
        );
        assert_eq!(
            measured_ups(
                &sample(90, true),
                &sample(180, false),
                Duration::from_secs(2)
            ),
            None
        );
        assert_eq!(
            measured_ups(
                &sample(90, false),
                &sample(0, false),
                Duration::from_secs(2)
            ),
            None
        );
    }
    #[test]
    fn target_history_preserves_speed_changes_pause_and_session_gaps() {
        let mut runtime = Runtime::default();
        let instant = Instant::now();
        runtime.accept(sample(0, false), instant, 10);
        let mut slower = sample(120, false);
        slower.speed = 0.7;
        runtime.accept(slower, instant + Duration::from_secs(2), 10);
        let mut paused = sample(120, true);
        paused.speed = 2.0;
        runtime.accept(paused, instant + Duration::from_secs(4), 10);
        runtime.end();
        runtime.accept(sample(240, false), instant + Duration::from_secs(6), 10);
        let snapshot = runtime.snapshot(true, 2);
        assert_eq!(snapshot.target_ups, Some(60.0));
        assert_eq!(
            snapshot
                .history
                .iter()
                .map(|point| point.target_ups)
                .collect::<Vec<_>>(),
            vec![Some(60.0), Some(42.0), Some(120.0), None, Some(60.0)]
        );
        assert!(matches!(snapshot.history[2].state, SampleState::Paused));
        assert!(matches!(
            snapshot.history[3].state,
            SampleState::Unavailable
        ));
        assert!(matches!(snapshot.history[4].state, SampleState::Collecting));
    }

    #[test]
    fn records_join_leave_and_resets_players_across_sessions() {
        let mut runtime = Runtime::default();
        runtime.observe_log("2026 [JOIN] Explorer joined the game");
        assert_eq!(runtime.players, Some(vec!["Explorer".into()]));
        runtime.observe_log("2026 [CHAT] Player: [LEAVE] Explorer left the game");
        assert_eq!(runtime.players, Some(vec!["Explorer".into()]));
        runtime.observe_log("2026 [LEAVE] Explorer left the game");
        assert_eq!(runtime.players, Some(Vec::new()));
        assert_eq!(runtime.activity.len(), 2);
        runtime.end();
        assert!(runtime.previous.is_none());
        assert_eq!(runtime.activity.len(), 2);
    }
    #[test]
    fn lost_samples_reset_delta_and_history_is_bounded() {
        let mut runtime = Runtime::default();
        let instant = Instant::now();
        runtime.accept(sample(0, false), instant, 10);
        runtime.accept(sample(120, false), instant + Duration::from_secs(2), 10);
        assert_eq!(runtime.ups, Some(60.0));
        runtime.unavailable("disconnected".into());
        runtime.accept(sample(240, false), instant + Duration::from_secs(4), 10);
        assert_eq!(runtime.ups, None);
        for _ in 0..250 {
            runtime.activity(ActivityKind::Joined, "test".into());
        }
        assert_eq!(runtime.activity.len(), 200);
    }
    #[tokio::test]
    async fn failed_reconciliation_keeps_live_samples_and_their_timestamps() {
        let mut calls = Vec::new();
        let observed = Instant::now();
        let (result, warning) = resilient_sample(true, |full| {
            calls.push(full);
            std::future::ready(if full {
                Err(Error::bad("Response exceeds size limit"))
            } else {
                Ok((sample(120, false), observed))
            })
        })
        .await;
        assert_eq!(calls, [true, false]);
        assert!(warning);
        assert!(result.is_ok_and(|(sample, time)| sample.tick == 120 && time == observed));
        calls.clear();
        let (result, warning) = resilient_sample(false, |full| {
            calls.push(full);
            std::future::ready(Ok(sample(240, false)))
        })
        .await;
        assert_eq!(calls, [false]);
        assert!(!warning);
        assert!(result.is_ok_and(|sample| sample.tick == 240));
    }
    #[test]
    fn reconciliation_detects_unqueued_research_across_lightweight_polls() {
        let mut runtime = Runtime::default();
        let instant = Instant::now();
        let mut baseline = sample(0, false);
        baseline.full_scan = true;
        baseline.completed.push(Completed {
            force: "player".into(),
            name: "triggered-tech".into(),
            level: 1,
            researched: false,
        });
        runtime.accept(baseline.clone(), instant, 10);
        runtime.accept(sample(120, false), instant + Duration::from_secs(2), 10);
        baseline.tick = 1800;
        baseline.completed[0].researched = true;
        runtime.accept(baseline.clone(), instant + Duration::from_secs(30), 10);
        baseline.tick = 3600;
        runtime.accept(baseline, instant + Duration::from_secs(60), 10);
        assert_eq!(
            runtime
                .activity
                .iter()
                .filter(|event| matches!(event.kind, ActivityKind::ResearchFinished))
                .count(),
            1
        );
        assert!(runtime.ups.is_some());
    }
    #[tokio::test]
    async fn fragmented_packets_round_trip_and_oversized_frame_is_rejected()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let sender = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let body = [
                10_i32.to_le_bytes(),
                2_i32.to_le_bytes(),
                0_i32.to_le_bytes(),
            ]
            .concat();
            for byte in body {
                stream.write_all(&[byte]).await?;
            }
            stream.write_all(&[0, 0]).await?;
            stream.write_all(&65537_i32.to_le_bytes()).await?;
            Ok::<(), std::io::Error>(())
        });
        let mut stream = TcpStream::connect(address).await?;
        let (id, kind, body) = read_packet(&mut stream)
            .await
            .map_err(|e| std::io::Error::other(e.message))?;
        assert_eq!((id, kind, body), (2, 0, Vec::new()));
        assert!(read_packet(&mut stream).await.is_err());
        sender.await??;
        Ok(())
    }
    #[test]
    fn completes_short_research_and_infinite_levels_without_cancellation_events() {
        let mut runtime = Runtime::default();
        let instant = Instant::now();
        let mut first = sample(0, false);
        first.completed.push(Completed {
            force: "player".into(),
            name: "automation".into(),
            level: 1,
            researched: false,
        });
        first.completed.push(Completed {
            force: "player".into(),
            name: "mining-productivity".into(),
            level: 10,
            researched: false,
        });
        runtime.accept(first.clone(), instant, 10);
        assert!(runtime.activity.is_empty());
        let mut second = first.clone();
        for technology in &mut second.completed {
            if technology.name == "automation" {
                technology.researched = true;
            } else {
                technology.level = 11;
            }
        }
        runtime.accept(second.clone(), instant + Duration::from_secs(2), 10);
        assert_eq!(runtime.activity.len(), 2);
        runtime.accept(second, instant + Duration::from_secs(4), 10);
        assert_eq!(runtime.activity.len(), 2);
        runtime.accept(first, instant + Duration::from_secs(6), 10);
        assert_eq!(runtime.activity.len(), 2);
    }
    #[test]
    fn decodes_factorio_empty_lua_tables() -> std::result::Result<(), serde_json::Error> {
        let sample: WireSample = serde_json::from_str(
            r#"{"tick":1,"speed":1,"paused":false,"players":{},"research":{},"completed":{}}"#,
        )?;
        assert!(sample.players.is_empty());
        assert!(sample.research.is_empty());
        assert!(serde_json::from_str::<WireSample>(r#"{"tick":1,"speed":1,"paused":false,"players":{"bad":true},"research":{},"completed":{}}"#).is_err());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "Requires FC_TEST_FACTORIO_BIN pointing to real headless Factorio"]
    async fn real_factorio_telemetry_reports_research_completion_and_pause()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        fn io(error: Error) -> std::io::Error {
            std::io::Error::other(error.message)
        }
        async fn script(session: &Session, code: &str) -> Result<()> {
            let mut stream = connect(session).await?;
            send_packet(&mut stream, 2, 2, &format!("/sc {code}; rcon.print('ok')")).await?;
            let (_, _, response) = read_packet(&mut stream).await?;
            if response != b"ok\n" && response != b"ok" {
                return Err(Error::bad(&format!(
                    "Test script failed: {}",
                    String::from_utf8_lossy(&response)
                )));
            }
            Ok(())
        }
        let binary = std::path::PathBuf::from(std::env::var("FC_TEST_FACTORIO_BIN")?);
        let directory = tempfile::tempdir()?;
        let app = App::new(directory.path().into(), Some(binary.clone())).map_err(io)?;
        let mut settings = storage::settings(&app.settings_path()).map_err(io)?;
        settings.settings.auto_pause = false;
        storage::update_settings(&app.settings_path(), settings).map_err(io)?;
        let config = app.directory.join("create.ini");
        storage::atomic_write(
            &config,
            format!(
                "[path]\nread-data=__PATH__executable__/../../data\nwrite-data={}\n",
                app.directory.display()
            )
            .as_bytes(),
            false,
        )
        .map_err(io)?;
        let output = tokio::process::Command::new(&binary)
            .arg("--config")
            .arg(&config)
            .arg("--create")
            .arg(app.saves_path().join("test.zip"))
            .output()
            .await?;
        assert!(output.status.success(), "isolated map creation failed");
        crate::process::start(&app, "test.zip").await.map_err(io)?;
        let monitor = tokio::spawn(crate::process::monitor(app.clone()));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if app.inner.lock().await.status.state == ServerState::Running {
                break;
            }
            assert!(Instant::now() < deadline, "server readiness timed out");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let session = app
            .inner
            .lock()
            .await
            .telemetry
            .session
            .clone()
            .ok_or_else(|| std::io::Error::other("missing test session"))?;
        let first = match read_sample(&session).await {
            Ok(sample) => sample,
            Err(_) => read_sample(&session).await.map_err(io)?,
        };
        assert!(first.players.is_empty());
        assert!(!first.paused);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let second = read_sample(&session).await.map_err(io)?;
        assert!(second.tick > first.tick);
        script(&session,"local f=game.forces.player; for _,t in pairs(f.technologies.automation.prerequisites) do t.researched=true end; assert(f.add_research('automation')); f.research_progress=0.4").await.map_err(io)?;
        let researching = read_sample(&session).await.map_err(io)?;
        assert!(
            researching
                .research
                .iter()
                .any(|r| r.name == "automation" && (r.progress - 0.4).abs() < 0.001)
        );
        let mut runtime = Runtime::default();
        runtime.accept(researching, Instant::now(), 10);
        for _ in 0..8 {
            script(&session, "game.forces.player.research_queue={}")
                .await
                .map_err(io)?;
            let canceled = read_sample(&session).await.map_err(io)?;
            assert!(
                canceled.watch.is_empty(),
                "Canceled entries must not accumulate in later polls"
            );
            assert!(
                session
                    .research_watch
                    .lock()
                    .map_err(|_| std::io::Error::other("watch lock"))?
                    .is_empty()
            );
            script(
                &session,
                "assert(game.forces.player.add_research('automation'))",
            )
            .await
            .map_err(io)?;
            let resumed = read_sample(&session).await.map_err(io)?;
            assert_eq!(resumed.watch.len(), 1);
        }
        script(
            &session,
            "game.forces.player.technologies.automation.researched=true",
        )
        .await
        .map_err(io)?;
        let completed = read_sample(&session).await.map_err(io)?;
        runtime.accept(completed, Instant::now(), 10);
        assert!(
            runtime
                .activity
                .iter()
                .any(|a| matches!(a.kind, ActivityKind::ResearchFinished)
                    && a.message.contains("automation"))
        );
        let mut reconciliation = Runtime::default();
        reconciliation.accept(
            read_sample_mode(&session, true).await.map_err(io)?,
            Instant::now(),
            10,
        );
        script(
            &session,
            "game.forces.player.technologies['steel-processing'].researched=true",
        )
        .await
        .map_err(io)?;
        reconciliation.accept(read_sample(&session).await.map_err(io)?, Instant::now(), 10);
        reconciliation.accept(
            read_sample_mode(&session, true).await.map_err(io)?,
            Instant::now(),
            10,
        );
        assert!(reconciliation.activity.iter().any(|event| matches!(
            event.kind,
            ActivityKind::ResearchFinished
        )
            && event.message.contains("steel-processing")));
        script(&session, "game.tick_paused=true")
            .await
            .map_err(io)?;
        assert!(read_sample(&session).await.map_err(io)?.paused);
        crate::process::shutdown(&app).await;
        monitor.abort();
        Ok(())
    }
}
