//! Un vrai téléphone simulé : connexion WebSocket au serveur, la console appliquant
//! les demandes comme le fait l'interface à chaque image.

use std::path::Path;
use std::time::{Duration, Instant};

use folie_dmx::NullOutput;
use folie_engine::{EngineConfig, OutputBinding};
use folie_fixture::{FixtureId, Library, PatchedFixture};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::*;

/// 2 MH 6, un groupe, une séquence de deux cues sur l'executor 1.
fn console() -> Console {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
    let (library, _) = Library::load_dir(root);
    let engine = folie_engine::spawn(
        EngineConfig { universes: 1, rate_hz: 200.0 },
        vec![OutputBinding { universe: 0, output: Box::new(NullOutput) }],
    );
    let mut console = Console::new(engine);
    for i in 0..2u32 {
        let fixture = PatchedFixture {
            id: FixtureId(i + 1),
            name: format!("F{}", i + 1),
            fixture_type: library.get("martin/rush-mh-6-wash").unwrap().clone(),
            mode: 0,
            universe: 0,
            address: 1 + 14 * i as u16,
        };
        console.show.patch.add(fixture).unwrap();
    }
    console.patch_changed();
    for line in
        ["Fixture 1 Thru 2", "Store Group 1 \"Face\"", "At 50", "Store Cue 1 Fade 0", "At 100", "Store Cue 2 Fade 0", "ClearAll"]
    {
        console.execute(line).unwrap_or_else(|e| panic!("{line} : {e}"));
    }
    console
}

type Phone = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn phone(port: u16, pin: &str) -> Phone {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws?pin={pin}")).await.unwrap();
    ws
}

async fn next_json(ws: &mut Phone) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(3), ws.next()).await.expect("rien reçu").unwrap().unwrap();
        if let WsMessage::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

async fn send(ws: &mut Phone, action: Value) {
    ws.send(WsMessage::Text(action.to_string().into())).await.unwrap();
}

/// Ce que fait l'interface à chaque image, jusqu'à ce que `done` soit vrai.
fn frames(server: &RemoteServer, console: &mut Console, mut done: impl FnMut(&Console, &[String]) -> bool) -> Vec<String> {
    let start = Instant::now();
    let mut announced = Vec::new();
    while start.elapsed() < Duration::from_secs(3) {
        for result in server.handle(console) {
            announced.push(result.map_or_else(|e| e.to_string(), |o| o.message));
        }
        std::thread::sleep(Duration::from_millis(10));
        server.publish(console, &console.engine().snapshot(), "Essai");
        if done(console, &announced) {
            return announced;
        }
    }
    panic!("délai dépassé ; annoncé : {announced:?}");
}

#[test]
fn phone_drives_the_console() {
    let mut console = console();
    let server = RemoteServer::start(0, "4821", "1357", || {}).unwrap();
    let port = server.port();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    // Code faux : refus expliqué, puis fermeture.
    let refused = rt.block_on(async {
        let mut ws = phone(port, "0000").await;
        next_json(&mut ws).await
    });
    assert_eq!(refused["type"], "refused");
    assert_eq!(refused["reason"], "pin");

    // La page est servie à tous (elle demande le code elle-même).
    let page = rt.block_on(async {
        let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut tcp, b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();
        let mut body = String::new();
        tokio::io::AsyncReadExt::read_to_string(&mut tcp, &mut body).await.unwrap();
        body
    });
    assert!(page.starts_with("HTTP/1.1 200") && page.contains("Folie-DMX"), "{page}");

    let mut ws = rt.block_on(phone(port, "4821"));
    let hello = rt.block_on(next_json(&mut ws));
    assert_eq!((hello["type"].as_str(), hello["role"].as_str()), (Some("hello"), Some("full")));
    frames(&server, &mut console, |_, _| server.clients() == 1);
    let state = rt.block_on(next_json(&mut ws));
    assert_eq!(state["type"], "state");
    assert_eq!(state["show"], "Essai");
    assert_eq!(state["page"], 1);
    assert_eq!(state["executors"].as_array().unwrap().len(), 20);
    assert_eq!(state["executors"][0]["kind"], "sequence");
    assert_eq!(state["executors"][0]["now"], "éteint");
    assert_eq!(state["executors"][0]["next"], "1");
    assert_eq!(state["executors"][1]["kind"], "empty");
    assert_eq!(state["groups"][0]["name"], "Face");

    // Fader puis Go : appliqués par la console, le fader sans bruit sur la ligne de commande.
    rt.block_on(send(&mut ws, json!({"type": "fader", "executor": 1, "level": 0.4})));
    rt.block_on(send(&mut ws, json!({"type": "press", "executor": 1})));
    let announced = frames(&server, &mut console, |c, _| {
        c.engine().snapshot().executors.get(&1).is_some_and(|s| s.cue == Some(0) && (s.level - 0.4).abs() < 1e-3)
    });
    assert_eq!(announced, ["Go executor 1"]);
    // Un état périodique peut passer avant la réponse sur une machine lente.
    let reply = rt.block_on(async {
        loop {
            let message = next_json(&mut ws).await;
            if message["type"] != "state" {
                break message;
            }
        }
    });
    assert_eq!((reply["type"].as_str(), reply["ok"].as_bool()), (Some("reply"), Some(true)), "{reply}");

    // L'état poussé suit : cue 1 en cours, fader à 40 %.
    let state = rt.block_on(async {
        loop {
            let message = next_json(&mut ws).await;
            if message["type"] == "state" && message["executors"][0]["now"] == "1" {
                break message;
            }
        }
    });
    assert!((state["executors"][0]["level"].as_f64().unwrap() - 0.4).abs() < 1e-3);

    // Une commande erronée : l'erreur revient au téléphone et s'affiche sur la console.
    rt.block_on(send(&mut ws, json!({"type": "command", "line": "Group 9"})));
    let announced = frames(&server, &mut console, |_, a| !a.is_empty());
    assert_eq!(announced.len(), 1);
    let reply = rt.block_on(async {
        loop {
            let message = next_json(&mut ws).await;
            if message["type"] == "reply" {
                break message;
            }
        }
    });
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["message"], announced[0]);

    // Flash tenu puis téléphone perdu : la touche est relâchée.
    console.set_executor_mode(1, folie_show::ExecutorMode::Flash).unwrap();
    rt.block_on(send(&mut ws, json!({"type": "flash", "executor": 1, "pressed": true})));
    frames(&server, &mut console, |c, _| c.engine().snapshot().executors.get(&1).is_some_and(|s| s.flashing));
    drop(ws);
    frames(&server, &mut console, |c, _| {
        server.clients() == 0 && c.engine().snapshot().executors.get(&1).is_none_or(|s| !s.flashing)
    });
}

#[test]
fn playback_code_only_plays_executors() {
    let mut console = console();
    let server = RemoteServer::start(0, "4821", "1357", || {}).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut ws = rt.block_on(phone(server.port(), "1357"));
    let hello = rt.block_on(next_json(&mut ws));
    assert_eq!(hello["role"], "playback");

    // Ligne de commande, fader, blackout : refusés par le serveur, sans atteindre la console.
    for action in [
        json!({"type": "command", "line": "ClearAll"}),
        json!({"type": "fader", "executor": 1, "level": 0.0}),
        json!({"type": "blackout", "on": true}),
        json!({"type": "off_all"}),
    ] {
        rt.block_on(send(&mut ws, action.clone()));
        let reply = rt.block_on(async {
            loop {
                let message = next_json(&mut ws).await;
                if message["type"] == "reply" {
                    break message;
                }
            }
        });
        assert_eq!(reply["ok"], false, "{action}");
    }
    // GO, lui, passe.
    rt.block_on(send(&mut ws, json!({"type": "go", "executor": 1})));
    let announced =
        frames(&server, &mut console, |c, _| c.engine().snapshot().executors.get(&1).is_some_and(|s| s.cue == Some(0)));
    assert_eq!(announced, ["Go executor 1"], "rien d'autre n'est arrivé à la console");
    let snapshot = console.engine().snapshot();
    assert!(!snapshot.blackout);
    assert!((snapshot.executors[&1].level - 1.0).abs() < 1e-3, "fader intact");
}

#[test]
fn stopping_the_server_frees_the_port() {
    let server = RemoteServer::start(0, "1", "2", || {}).unwrap();
    let port = server.port();
    drop(server);
    let again = RemoteServer::start(port, "1", "2", || {}).expect("port libéré");
    assert_eq!(again.port(), port);
}

/// Un client OSC (comme TouchOSC) : fader et Go, puis le retour d'état sur son port.
#[test]
fn osc_client_drives_the_console_and_gets_feedback() {
    use rosc::{OscMessage, OscPacket, OscType};

    let mut console = console();
    let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    // Port de retour 0 : réponse au port d'où le client écrit.
    let server = osc::OscServer::start(0, 0, vec![], true, || {}).unwrap();
    let send = |addr: &str, args: Vec<OscType>| {
        let packet = OscPacket::Message(OscMessage { addr: addr.into(), args });
        client.send_to(&rosc::encoder::encode(&packet).unwrap(), ("127.0.0.1", server.port())).unwrap();
    };
    send("/folie/exec/1/fader", vec![OscType::Float(0.4)]);
    send("/folie/exec/1/go", vec![OscType::Float(1.0)]);
    send("/folie/exec/1/go", vec![OscType::Float(0.0)]); // relâchement : rien
    let start = Instant::now();
    let mut announced = Vec::new();
    while start.elapsed() < Duration::from_secs(3) {
        for result in server.handle(&mut console) {
            announced.push(result.map_or_else(|e| e.to_string(), |o| o.message));
        }
        let status = console.engine().snapshot().executors.get(&1).cloned();
        if status.is_some_and(|s| s.cue == Some(0) && (s.level - 0.4).abs() < 1e-3) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(announced, ["Go executor 1"], "un seul Go, le fader sans bruit");
    assert_eq!(server.clients(), 1);

    // Retour d'état : le client reçoit le fader et la cue en cours.
    server.publish(&console, &console.engine().snapshot());
    let mut buffer = [0u8; rosc::decoder::MTU];
    let mut received = Vec::new();
    client.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    while let Ok((size, _)) = client.recv_from(&mut buffer) {
        assert!(size < 1500, "paquet trop gros : {size} octets");
        let (_, packet) = rosc::decoder::decode_udp(&buffer[..size]).unwrap();
        let OscPacket::Bundle(bundle) = packet else { panic!("paquet groupé attendu") };
        received.extend(bundle.content);
    }
    let value = |addr: &str| {
        received.iter().find_map(|p| match p {
            OscPacket::Message(m) if m.addr == addr => m.args.first().cloned(),
            _ => None,
        })
    };
    assert!(matches!(value("/folie/exec/1/fader"), Some(OscType::Float(f)) if (f - 0.4).abs() < 1e-3));
    assert_eq!(value("/folie/exec/1/cue"), Some(OscType::String("1".into())));
    assert_eq!(value("/folie/exec/1/on"), Some(OscType::Int(1)));
    assert_eq!(value("/folie/page"), Some(OscType::Int(1)));

    // Reaper passe le marqueur 2, puis le redit : Goto Cue 2, une seule fois.
    for _ in 0..2 {
        send("/lastmarker/number/str", vec![OscType::String("2".into())]);
    }
    let start = Instant::now();
    let mut announced = Vec::new();
    while start.elapsed() < Duration::from_secs(3) && announced.is_empty() {
        for result in server.handle(&mut console) {
            announced.push(result.map_or_else(|e| e.to_string(), |o| o.message));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert!(server.handle(&mut console).is_empty(), "le même marqueur ne repart pas");
    assert_eq!(announced, ["Goto cue 2"]);
    // Retour d'état de ce changement, une fois la cue 2 atteinte.
    while console.engine().snapshot().executors.get(&1).and_then(|s| s.cue) != Some(1) {
        std::thread::sleep(Duration::from_millis(5));
    }
    server.publish(&console, &console.engine().snapshot());
    while client.recv_from(&mut buffer).is_ok() {}

    // Rien n'a changé : rien n'est renvoyé.
    server.publish(&console, &console.engine().snapshot());
    assert!(client.recv_from(&mut buffer).is_err());
}
