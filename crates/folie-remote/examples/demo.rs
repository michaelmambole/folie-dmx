//! Télécommande sans l'interface, pour travailler la page : un petit show de démonstration.
//!
//! `cargo run -p folie-remote --example demo` puis http://localhost:8090 (code régie 1234, code lecture 5678).

use std::path::Path;
use std::time::{Duration, Instant};

use folie_control::Console;
use folie_dmx::NullOutput;
use folie_engine::{EngineConfig, OutputBinding};
use folie_fixture::{FixtureId, Library, PatchedFixture};
use folie_remote::RemoteServer;

fn main() {
    env_logger::init();
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
    let (library, _) = Library::load_dir(root);
    let engine = folie_engine::spawn(EngineConfig::default(), vec![OutputBinding { universe: 0, output: Box::new(NullOutput) }]);
    let mut console = Console::new(engine);
    for i in 0..4u32 {
        let fixture = PatchedFixture {
            id: FixtureId(i + 1),
            name: format!("MH6 {}", i + 1),
            fixture_type: library.get("martin/rush-mh-6-wash").unwrap().clone(),
            mode: 0,
            universe: 0,
            address: 1 + 14 * i as u16,
        };
        console.show.patch.add(fixture).unwrap();
    }
    console.patch_changed();
    let lines = [
        "Fixture 1 Thru 4",
        "Store Group 1 \"Tous\"",
        "Fixture 1 + 2",
        "Store Group 2 \"Cour\"",
        "Fixture 3 + 4",
        "Store Group 3 \"Jardin\"",
        "Fixture 1 Thru 4 At 60",
        "Store Cue 1 Fade 3 \"Entrée\"",
        "At 100",
        "Store Cue 2 Fade 5 \"Plein feu\"",
        "At 20",
        "Store Cue 3 \"Nuit\"",
        "ClearAll",
        "Group 2 At 100",
    ];
    for line in lines {
        console.execute(line).unwrap_or_else(|e| panic!("{line} : {e}"));
    }
    console.store_in_executor(2).unwrap();
    console.execute("ClearAll").unwrap();
    console.execute("Group 3 At 100").unwrap();
    console.store_in_executor(11).unwrap();
    console.set_executor_mode(11, folie_show::ExecutorMode::Flash).unwrap();
    console.execute("ClearAll").unwrap();
    console.rename_page(1, "Acte 1".into());
    console.select_executor(1);

    let server = RemoteServer::start(8090, "1234", "5678", || {}).expect("port 8090");
    println!("http://localhost:{}  (code régie 1234, code lecture 5678)", server.port());
    let mut last = Instant::now();
    loop {
        for result in server.handle(&mut console) {
            match result {
                Ok(o) => println!("{}", o.message),
                Err(e) => println!("erreur : {e}"),
            }
        }
        if last.elapsed() > Duration::from_millis(30) {
            server.publish(&console, &console.engine().snapshot(), "Démo");
            last = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
