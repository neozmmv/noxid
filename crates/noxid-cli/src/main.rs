//! Development harness for Noxid: drives the core without the UI.
//!
//! `noxid-cli call` starts a node and reads commands from stdin, one per line.
//! Run several on one machine by giving each its own `NOXID_HOME`.

use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use noxid_core::media::{self, Mic, Screen};
use noxid_core::friends::{FriendsView, parse_friend_id};
use noxid_core::{Node, NodeConfig, Notice, QualityMode, Snapshot, Stream};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser)]
#[command(name = "noxid-cli", about = "Noxid development harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a node and control it with commands typed on stdin (`help` lists
    /// them).
    Call {
        /// The name others see.
        #[arg(long, default_value = "")]
        name: String,
        /// Send a generated tone instead of the real microphone.
        #[arg(long)]
        test_mic: bool,
        /// Start with the microphone muted.
        #[arg(long)]
        muted: bool,
        /// Print the room every second instead of only when it changes.
        #[arg(long)]
        stats: bool,
    },
    /// List the displays and windows this machine can share.
    Devices,
    /// Try to subscribe to a node's media directly, bypassing membership.
    /// Should fail unless this node is in their room: it checks the gate.
    Probe {
        /// The target's endpoint id.
        id: iroh::EndpointId,
    },
}

const HELP: &str = "\
commands:
  create [room name]        create a room (you become its host)
  invite                    mint a single-use invite code
  join <code>               join with an invite, or knock with a room code
  approve <id|name>         let someone in who is knocking
  deny <id|name>            turn them away
  kick <id|name>            remove a member (host only)
  leave                     leave the room (the host leaving closes it)
  mic                       toggle the microphone
  share [display|test]      share a screen (index, id or name from `devices`)
  unshare                   stop sharing
  max <low|mid|high>        cap what you share
  fps <24|30|60>            how many pictures a second you share
  bitrate <kbps|auto>       the top quality's bitrate (auto: its default)
  quality <who> <auto|low|mid|high>   how to receive someone's share
  devices                   list microphones and outputs
  input <id|default>        pick the microphone
  output <id|default>       pick where others are heard
  pcsound <on|off>          include the PC's sound when sharing
  volume <who> <voice|screen> <percent> [muted]   how loud you hear someone (voice up to 200, screen up to 100)
  status                    print the room
  id                        print your Noxid ID (what friends add you by)
  friends                   list friends, requests and rings
  friend add <id>           send a friend request
  friend accept <who>       accept a request (who: id, id prefix or name)
  friend decline <who>      decline a request
  friend cancel <who>       take back your request
  friend remove <who>       end a friendship
  friend retry <who>        try delivering to them now
  friend call <who>         ring a friend into your room (starts one if needed)
  friend hangup             stop ringing
  friend pickup <who>       answer a friend ringing you
  friend reject <who>       decline a ring
  quit";

#[tokio::main]
async fn main() -> Result<()> {
    noxid_core::log::init(noxid_core::log::path());

    match Cli::parse().command {
        Command::Call {
            name,
            test_mic,
            muted,
            stats,
        } => call(name, test_mic, muted, stats).await,
        Command::Devices => devices().await,
        Command::Probe { id } => probe(id).await,
    }
}

async fn probe(id: iroh::EndpointId) -> Result<()> {
    let live = iroh_live::Live::from_env().await?.spawn();
    let path = noxid_core::voice_path(&id);
    let result = tokio::time::timeout(Duration::from_secs(15), live.subscribe(id, &path)).await;
    match result {
        Ok(Ok(_)) => println!("PROBE: subscribed; the gate let a non-member in"),
        Ok(Err(e)) => println!("PROBE: refused ({e:#})"),
        Err(_) => println!("PROBE: refused (timed out)"),
    }
    live.shutdown().await;
    Ok(())
}

async fn devices() -> Result<()> {
    let displays = media::displays().await?;
    if displays.is_empty() {
        println!("no displays found");
    }
    for (index, d) in displays.iter().enumerate() {
        println!(
            "{index}: {} ({}x{})\n   id: {}",
            d.name, d.size.width, d.size.height, d.id
        );
    }
    let windows = media::windows().await;
    if !windows.is_empty() {
        println!("\nwindows (share by id):");
    }
    for w in windows {
        println!(
            "  {} [{}] ({}x{})\n   id: {}",
            w.name, w.app, w.size.width, w.size.height, w.id
        );
    }
    Ok(())
}

async fn call(name: String, test_mic: bool, muted: bool, stats: bool) -> Result<()> {
    let node = Node::spawn(NodeConfig {
        secret_key: noxid_core::identity::load_or_create()?,
        name,
        mic: if test_mic { Mic::Test } else { Mic::Device },
        start_muted: muted,
        mic_device: None,
        output_device: None,
    })
    .await?;
    println!("node {} ready; type `help`", node.id());

    let mut changes = node.changes();
    let mut notices = node.notices();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut last_printed = String::new();
    let mut stdin_open = true;

    loop {
        tokio::select! {
            line = lines.next_line(), if stdin_open => match line? {
                Some(line) => {
                    let line = line.trim().to_string();
                    if line == "quit" {
                        break;
                    }
                    if let Err(e) = run_command(&node, &line).await {
                        println!("error: {e:#}");
                    }
                }
                // Stdin closed (driven by a script): keep running until killed.
                None => stdin_open = false,
            },
            notice = notices.recv() => {
                if let Ok(notice) = notice {
                    match notice {
                        Notice::Kicked => println!("** you were removed from the room"),
                        Notice::RoomClosed => println!("** the host closed the room"),
                        Notice::Error(e) => println!("** {e}"),
                        Notice::Friend(event) => println!("** friends: {event:?}"),
                    }
                }
            }
            _ = changes.changed() => {}
            _ = tick.tick() => {}
            _ = tokio::signal::ctrl_c() => break,
        }

        // Rosters and knocks are printed when they change; per-peer stats only
        // with --stats, or the terminal would scroll every second.
        let snapshot = node.snapshot();
        let summary = render(&snapshot, stats, &node.levels());
        if summary != last_printed {
            println!("{summary}");
            last_printed = summary;
        }
    }

    node.shutdown().await;
    Ok(())
}

async fn run_command(node: &Node, line: &str) -> Result<()> {
    let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
    let rest = rest.trim();
    match command {
        "" => {}
        "help" => println!("{HELP}"),
        "create" => {
            node.create((!rest.is_empty()).then(|| rest.to_string())).await?;
        }
        "invite" => println!("invite: {}", node.invite(None)?),
        "join" => {
            println!("joining (a knock waits until the host answers)...");
            node.join(rest).await?;
            println!("joined");
        }
        "approve" | "deny" => {
            let id = knock_id(node, rest)?;
            node.answer_knock(&id, command == "approve")?;
        }
        "kick" => node.kick(rest).await?,
        "leave" => node.leave().await,
        "mic" => node.set_mic(!node.snapshot().mic_on),
        "share" => {
            let screen = match rest {
                "test" => Screen::Test,
                "" => Screen::Display(media::find_display(None).await?),
                spec => Screen::Display(media::find_display(Some(spec)).await?),
            };
            node.share_screen(Some(screen))?;
        }
        "unshare" => node.share_screen(None)?,
        "max" => node.set_max_quality(rest.parse()?)?,
        "fps" => node.set_framerate(rest.parse()?)?,
        "bitrate" => node.set_bitrate(match rest {
            "auto" | "" => None,
            kbps => Some(kbps.parse::<u64>()? * 1000),
        })?,
        "quality" => {
            let (who, mode) = rest
                .rsplit_once(' ')
                .ok_or_else(|| anyhow::anyhow!("usage: quality <who> <auto|low|mid|high>"))?;
            let mode = match mode {
                "auto" => QualityMode::Auto,
                q => QualityMode::Pinned(q.parse()?),
            };
            node.set_peer_quality(member_id(node, who)?.parse()?, mode);
        }
        "status" => println!("{}", render(&node.snapshot(), true, &node.levels())),
        "devices" => {
            let (inputs, outputs) = noxid_core::audio::devices().await;
            println!("microphones:");
            for d in inputs {
                println!("  {}{}
    id: {}", d.name, if d.default { " (default)" } else { "" }, d.id);
            }
            println!("outputs:");
            for d in outputs {
                println!("  {}{}
    id: {}", d.name, if d.default { " (default)" } else { "" }, d.id);
            }
        }
        "input" => node.set_mic_device(device_arg(rest)),
        "output" => node.set_output_device(device_arg(rest)).await?,
        "pcsound" => node.set_screen_audio(rest == "on"),
        "volume" => {
            let mut parts = rest.split_whitespace();
            let (Some(who), Some(stream), Some(percent)) = (parts.next(), parts.next(), parts.next()) else {
                anyhow::bail!("usage: volume <who> <voice|screen> <0-200> [muted]");
            };
            let stream = match stream {
                "voice" => Stream::Voice,
                "screen" => Stream::Screen,
                other => anyhow::bail!("{other:?} is neither voice nor screen"),
            };
            let volume = percent.parse::<f32>()? / 100.0;
            let muted = parts.next() == Some("muted");
            node.set_peer_audio(member_id(node, who)?.parse()?, stream, volume, muted);
        }
        "id" => println!("{}", node.id()),
        "friends" => print_friends(&node.friends().view()),
        "friend" => friend_command(node, rest).await?,
        other => println!("unknown command {other:?}; type `help`"),
    }
    Ok(())
}

async fn friend_command(node: &Node, line: &str) -> Result<()> {
    let (command, who) = line.split_once(' ').unwrap_or((line, ""));
    let who = who.trim();
    let friends = node.friends();
    match command {
        "add" => {
            friends.send_request(parse_friend_id(who, node.id())?).await?;
            println!("request saved; it is delivered when you are both online");
        }
        "accept" => friends.answer(friend_peer(node, who)?, true).await?,
        "decline" => friends.answer(friend_peer(node, who)?, false).await?,
        "cancel" => friends.cancel(friend_peer(node, who)?).await?,
        "remove" => friends.remove(friend_peer(node, who)?).await?,
        "retry" => friends.retry(friend_peer(node, who)?),
        "call" => {
            let peer = friend_peer(node, who)?;
            let node = node.clone();
            // In the background, so `friend hangup` can still be typed.
            tokio::spawn(async move {
                match node.call_friend(peer).await {
                    Ok(outcome) => println!("** call: {outcome:?}"),
                    Err(e) => println!("** call failed: {e:#}"),
                }
            });
        }
        "hangup" => friends.hang_up(),
        "pickup" => node.answer_ring(friend_peer(node, who)?, true).await?,
        "reject" => node.answer_ring(friend_peer(node, who)?, false).await?,
        other => anyhow::bail!("unknown friend command {other:?}; type `help`"),
    }
    Ok(())
}

/// Resolves someone on the friends list by id, id prefix or name, or reads
/// a full Noxid ID.
fn friend_peer(node: &Node, query: &str) -> Result<iroh::EndpointId> {
    anyhow::ensure!(!query.is_empty(), "who?");
    let view = node.friends().view();
    let known = view
        .friends
        .iter()
        .map(|f| (&f.id, &f.name))
        .chain(view.incoming.iter().map(|r| (&r.id, &r.name)))
        .chain(view.outgoing.iter().map(|r| (&r.id, &r.name)))
        .chain(view.rings.iter().map(|r| (&r.id, &r.name)));
    for (id, name) in known {
        if id.starts_with(query) || name.eq_ignore_ascii_case(query) {
            return Ok(id.parse()?);
        }
    }
    parse_friend_id(query, node.id())
}

fn print_friends(view: &FriendsView) {
    if let Some(problem) = &view.problem {
        println!("  (read-only: {problem})");
    }
    for f in &view.friends {
        let state = if f.finishing { "finishing".to_string() } else { format!("{:?}", f.presence).to_lowercase() };
        println!("  friend    {} {} [{state}]", short(&f.id), f.name);
    }
    for r in &view.incoming {
        println!("  incoming  {} {}", short(&r.id), r.name);
    }
    for r in &view.outgoing {
        let state = match (&r.delivered_at, &r.last_error) {
            (Some(_), _) => "delivered".to_string(),
            (None, Some(e)) => format!("not delivered: {e}"),
            (None, None) => "sending".to_string(),
        };
        println!("  outgoing  {} {} [{state}]", short(&r.id), r.name);
    }
    for r in &view.rings {
        println!("  ringing   {} {}", short(&r.id), r.name);
    }
    if let Some(c) = &view.calling {
        println!("  calling   {} {}", short(&c.id), c.name);
    }
    for m in &view.missed {
        println!("  missed    {} {}", short(&m.id), m.name);
    }
    if view.friends.is_empty() && view.incoming.is_empty() && view.outgoing.is_empty() {
        println!("  no friends yet; `id` prints yours to share");
    }
}

fn short(id: &str) -> &str {
    &id[..id.len().min(10)]
}

/// `default` (or nothing) means the system default device.
fn device_arg(arg: &str) -> Option<String> {
    (!arg.is_empty() && arg != "default").then(|| arg.to_string())
}

/// Resolves a knocker by id, short id or name.
fn knock_id(node: &Node, query: &str) -> Result<String> {
    let snapshot = node.snapshot();
    snapshot
        .knocks
        .iter()
        .find(|k| k.id == query || k.id.starts_with(query) || k.name.eq_ignore_ascii_case(query))
        .map(|k| k.id.clone())
        .ok_or_else(|| anyhow::anyhow!("nobody matching {query:?} is knocking"))
}

/// Resolves a member by id, short id or name.
fn member_id(node: &Node, query: &str) -> Result<String> {
    let snapshot = node.snapshot();
    snapshot
        .room
        .iter()
        .flat_map(|r| &r.members)
        .find(|m| m.id == query || m.id.starts_with(query) || m.name.eq_ignore_ascii_case(query))
        .map(|m| m.id.clone())
        .ok_or_else(|| anyhow::anyhow!("nobody matching {query:?} is in the room"))
}

fn render(s: &Snapshot, stats: bool, levels: &std::collections::HashMap<String, f32>) -> String {
    let mut out = String::new();
    let me = format!(
        "[{}] mic {}, sharing {}",
        s.name,
        if s.mic_on { "on" } else { "muted" },
        if s.sharing { "on" } else { "off" }
    );
    let Some(room) = &s.room else {
        return format!("{me}; not in a room");
    };
    out.push_str(&format!(
        "{me}; room {}{}\n",
        room.name.as_deref().unwrap_or("(unnamed)"),
        if room.is_coordinator { " (you host)" } else { "" }
    ));
    if let Some(code) = &room.room_code {
        out.push_str(&format!("  room code: {code}\n"));
    }
    for m in &room.members {
        let short: String = m.id.chars().take(10).collect();
        let mut line = format!(
            "  {short} {:<12}{}",
            m.name,
            if m.is_coordinator { " host" } else { "     " }
        );
        if m.is_self {
            line.push_str(" (you)");
        } else if !m.peer.connected {
            line.push_str(" connecting...");
        } else {
            line.push_str(&format!(
                " {} | {}{}",
                if m.peer.has_audio { "audio" } else { "muted" },
                if m.peer.has_video { "sharing" } else { "not sharing" },
                if m.peer.has_screen_audio { " with sound" } else { "" },
            ));
            if stats {
                let level = levels.get(&m.id).copied().unwrap_or(0.0);
                let bars = (level * 10.0).round() as usize;
                line.push_str(&format!(" | level {:<10}", "#".repeat(bars)));
                if let Some(r) = &m.peer.rendition {
                    line.push_str(&format!(
                        " | {r} {}x{} {} fps",
                        m.peer.width, m.peer.height, m.peer.fps
                    ));
                }
                if let Some(rtt) = m.peer.rtt_ms {
                    line.push_str(&format!(" | rtt {rtt}ms"));
                }
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    for k in &s.knocks {
        let short: String = k.id.chars().take(10).collect();
        out.push_str(&format!(
            "  ** {} ({short}) is knocking: approve {short} / deny {short}\n",
            k.name
        ));
    }
    out.trim_end().to_string()
}
