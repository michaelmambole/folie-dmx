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
    // La réponse et l'état poussé (cue 1 en cours, fader à 40 %) arrivent dans un ordre
    // qui dépend de la machine ; l'état n'est poussé qu'une fois, il ne faut pas le jeter.
    let (reply, state) = rt.block_on(async {
        let (mut reply, mut state) = (None, None);
        while reply.is_none() || state.is_none() {
            let message = next_json(&mut ws).await;
            if message["type"] == "reply" {
                reply = Some(message);
            } else if message["type"] == "state" && message["executors"][0]["now"] == "1" {
                state = Some(message);
            }
        }
        (reply.unwrap(), state.unwrap())
    });
    assert_eq!(reply["ok"].as_bool(), Some(true), "{reply}");
    assert!((state["executors"][0]["level"].as_f64().unwrap() - 0.4).abs() < 1e-3);

    // Une commande erronée : l'erreur revient au téléphone et s'affiche sur la console.
    rt.block_on(send(&mut ws, json!({"type": "command", "line": "Group 9"})));
    let announced = frames(&server, &mut console, |_, a| !a.is_empty());
    assert_eq!(announced.len(), 1);
    // Les réponses au fader et au Go peuvent arriver encore maintenant : on attend celle
    // de cette commande.
    let reply = rt.block_on(async {
        loop {
            let message = next_json(&mut ws).await;
            if message["type"] == "reply" && message["ok"] == false {
                break message;
            }
        }
    });
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

/// Onglet Sons : la liste et ce qui joue, lancer et arrêter ; le code lecture n'y a
/// droit que si la console le permet.
#[test]
fn phone_plays_sounds() {
    use folie_control::sounds::SoundRequest;
    use folie_show::media::Sound;

    let mut console = console();
    console.show.sounds.insert(1, Sound { name: "Pluie".into(), start: 2.0, then: Some(2), ..Default::default() });
    console.show.sounds.insert(2, Sound { name: "Orage".into(), group: 1, ..Default::default() });
    let server = RemoteServer::start(0, "4821", "1357", || {}).unwrap();
    server.set_sounds(state::SoundsLive {
        positions: [(1, 3.53)].into(),
        durations: [(1, 30.0), (2, 12.25)].into(),
    });
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut ws = rt.block_on(phone(server.port(), "1357"));
    assert_eq!(rt.block_on(next_json(&mut ws))["role"], "playback");
    frames(&server, &mut console, |_, _| server.clients() == 1);
    let state = rt.block_on(next_json(&mut ws));
    assert_eq!(state["playback_sounds"], false);
    assert_eq!(state["sounds"][0]["name"], "Pluie");
    assert_eq!(state["sounds"][0]["then"], 2);
    assert_eq!(state["sounds"][0]["elapsed"], 1.5, "depuis le point d'entrée, au dixième");
    assert_eq!(state["sounds"][0]["duration"], 30.0);
    assert_eq!(state["sounds"][1]["group"], 1);
    assert!(state["sounds"][1]["elapsed"].is_null(), "à l'arrêt");

    // Code lecture : refusé, puis permis par la console.
    let reply = |ws: &mut Phone| {
        rt.block_on(async {
            loop {
                let message = next_json(ws).await;
                if message["type"] == "reply" {
                    break message;
                }
            }
        })
    };
    rt.block_on(send(&mut ws, json!({"type": "sound", "number": 2})));
    assert_eq!(reply(&mut ws)["ok"], false);
    server.set_playback_sounds(true);
    rt.block_on(send(&mut ws, json!({"type": "sound", "number": 2})));
    rt.block_on(send(&mut ws, json!({"type": "sound_off", "number": 1})));
    rt.block_on(send(&mut ws, json!({"type": "sound_off", "number": null})));
    let announced = frames(&server, &mut console, |_, a| a.len() == 3);
    assert_eq!(announced, ["Son 2 « Orage » lancé", "Son 1 « Pluie » arrêté", "Tous les sons arrêtés"]);
    assert_eq!(
        console.take_sound_requests(),
        [SoundRequest::Play(2), SoundRequest::Stop { number: 1, fade: 1.0 }, SoundRequest::StopAll { fade: 0.0 }]
    );
    let state = rt.block_on(async {
        loop {
            let message = next_json(&mut ws).await;
            if message["type"] == "state" {
                break message;
            }
        }
    });
    assert_eq!(state["playback_sounds"], true);
}

/// Blind, Tap tempo, programmer et fonction des faders : ce que la console affiche,
/// le téléphone l'affiche aussi.
#[test]
fn phone_sees_blind_tempo_programmer_and_fader_functions() {
    let mut console = console();
    console.show.executors.get_mut(&1).unwrap().options.fader = folie_show::FaderFunction::Speed;
    console.show.executors.get_mut(&1).unwrap().options.color = Some([76, 175, 80]);
    console.execute("Fixture 1 At 50").unwrap();
    console.set_tempo(60.0);
    let server = RemoteServer::start(0, "4821", "1357", || {}).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut ws = rt.block_on(phone(server.port(), "4821"));
    rt.block_on(next_json(&mut ws));
    frames(&server, &mut console, |_, _| server.clients() == 1);
    let state = rt.block_on(next_json(&mut ws));
    assert_eq!(state["tempo"], 60.0);
    assert_eq!((state["blind"].as_bool(), state["programmer"].as_u64()), (Some(false), Some(1)));
    assert_eq!(state["executors"][0]["fader_function"], "Vitesse");
    assert_eq!(state["executors"][0]["value"], "× 2.00", "fader en haut : vitesse doublée");
    assert_eq!(state["executors"][0]["color"], "#4caf50");
    assert!(state["executors"][1]["color"].is_null());

    // Blind, puis deux frappes de Tap à 0,4 s : 150 BPM.
    rt.block_on(send(&mut ws, json!({"type": "blind", "on": true})));
    frames(&server, &mut console, |c, _| c.blind());
    rt.block_on(send(&mut ws, json!({"type": "tap"})));
    // La première frappe doit être arrivée à la console (sa réponse revient) avant la seconde.
    let mut first = None;
    while first.is_none() {
        frames(&server, &mut console, |_, _| true);
        first = rt.block_on(async {
            loop {
                match tokio::time::timeout(Duration::from_millis(50), ws.next()).await {
                    Ok(Some(Ok(WsMessage::Text(text)))) => {
                        let message: Value = serde_json::from_str(&text).unwrap();
                        if message["type"] == "reply" && message["message"].as_str().is_some_and(|m| m.starts_with("Tap")) {
                            break Some(message);
                        }
                    }
                    Ok(_) => continue,
                    Err(_) => break None,
                }
            }
        });
    }
    assert_eq!(first.unwrap()["message"], "Tap : encore une frappe");
    std::thread::sleep(Duration::from_millis(400));
    rt.block_on(send(&mut ws, json!({"type": "tap"})));
    // Entre les deux frappes, 0,4 s et un peu plus selon la machine : 130 à 150 BPM.
    let announced = frames(&server, &mut console, |c, _| (110.0..=160.0).contains(&c.tempo()));
    assert!(announced.is_empty(), "le Tap ne s'annonce pas sur la console : {announced:?}");
    let state = rt.block_on(async {
        loop {
            let message = next_json(&mut ws).await;
            if message["type"] == "state" && message["blind"] == true && message["tempo"].as_f64().unwrap() > 100.0 {
                break message;
            }
        }
    });
    assert_eq!(state["programmer"], 0, "le programmer blind est vide");
    assert!((110.0..=160.0).contains(&state["tempo"].as_f64().unwrap()), "{}", state["tempo"]);
}

/// Onglet Commande : presets par famille et macros ; timecode reçu et séquence qui le suit.
#[test]
fn phone_sees_presets_macros_and_timecode() {
    let mut console = console();
    for line in ["Fixture 1 Thru 2 At 30", r#"Store Preset 1.2 "Lueur""#, "ClearAll"] {
        console.execute(line).unwrap_or_else(|e| panic!("{line} : {e}"));
    }
    console.show.macros.insert(3, folie_show::Macro { name: "Salut".into(), lines: vec!["Go".into()] });
    console.select_executor(1);
    console.set_cue_timecode(folie_show::CueNumber::whole(2), Some(12.0)).unwrap();
    console.timecode_tick(83.47);
    let server = RemoteServer::start(0, "4821", "1357", || {}).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut ws = rt.block_on(phone(server.port(), "4821"));
    rt.block_on(next_json(&mut ws));
    frames(&server, &mut console, |_, _| server.clients() == 1);
    let state = rt.block_on(next_json(&mut ws));
    assert_eq!(state["presets"][0], json!({"feature": "Dimmer", "key": "1.2", "name": "Lueur"}));
    assert_eq!(state["macros"][0], json!({"number": 3, "name": "Salut", "running": false}));
    assert_eq!(state["timecode"], 83.4, "au dixième, sans arrondir vers le haut");
    assert_eq!(state["executors"][0]["follows"], "TC", "une cue à temps TC, pas de son suivi");
    assert_eq!(state["executors"][1]["follows"], "");
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

/// Un contrôleur MIDI : CC 7 sur le fader 1, note 36 sur Go 1, apprentissage, retour d'état.
#[test]
fn midi_controller_drives_executors() {
    use crate::midi::{Control, Mapping, MidiServer, MidiSettings, Target, feedback};
    let mut console = console();
    let fader = Control::Cc { channel: 0, number: 7 };
    let go = Control::Note { channel: 0, number: 36 };
    let settings = MidiSettings {
        enabled: true,
        // Aucun vrai port dans les tests : les messages sont injectés.
        inputs: vec!["(aucun port)".into()],
        mappings: vec![
            Mapping { control: fader, target: Target::Fader { slot: 1 } },
            Mapping { control: go, target: Target::Go { slot: 1 } },
        ],
        ..Default::default()
    };
    let midi = MidiServer::start(&settings).unwrap();
    assert!(midi.opened.is_empty());

    midi.inject(&[0xB0, 7, 51]); // 40 %
    midi.inject(&[0x90, 36, 100]); // Go
    midi.inject(&[0x80, 36, 0]); // relâché : rien
    let handled = midi.handle(&mut console);
    let announced: Vec<String> = handled.results.into_iter().map(|r| r.unwrap().message).collect();
    assert_eq!(announced, ["Go executor 1"], "le fader ne s'annonce pas");
    let start = Instant::now();
    let status = loop {
        let snapshot = console.engine().snapshot();
        if let Some(s) = snapshot.executors.get(&1).filter(|s| s.cue == Some(0)) {
            break s.clone();
        }
        assert!(start.elapsed() < Duration::from_secs(3), "Go jamais arrivé");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!((status.level - 51.0 / 127.0).abs() < 1e-3);

    // Retour : fader à sa valeur, LED du Go allumée (l'executor joue).
    let values = feedback(&settings.mappings, &console, &console.engine().snapshot());
    assert_eq!(values, [(fader, 51, None), (go, 127, None)]);
    // L'executor a une couleur de repère : elle accompagne la LED (pads RGB).
    console.show.executors.get_mut(&1).unwrap().options.color = Some([229, 72, 77]);
    let values = feedback(&settings.mappings, &console, &console.engine().snapshot());
    assert_eq!(values[1], (go, 127, Some([229, 72, 77])));

    // Apprentissage : le prochain appui est rendu au lieu d'agir.
    midi.learn(true);
    midi.inject(&[0x99, 40, 90]);
    let handled = midi.handle(&mut console);
    assert_eq!(handled.learned, Some(Control::Note { channel: 9, number: 40 }));
    assert!(handled.results.is_empty());

    // MSC : GO cue 2 de la liste 1.
    midi.inject(&[0xF0, 0x7F, 0x7F, 0x02, 0x01, 0x01, b'2', 0x00, b'1', 0xF7]);
    let handled = midi.handle(&mut console);
    assert_eq!(handled.results.into_iter().map(|r| r.unwrap().message).collect::<Vec<_>>(), ["Goto cue 2"]);
}
