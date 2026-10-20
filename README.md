# Folie-DMX

Logiciel de contrôle lumière inspiré de grandMA2 onPC, écrit en Rust,
avec sortie sur Enttec DMX USB Pro.

## Structure

| Crate | Rôle |
|---|---|
| `crates/folie-dmx` | Types DMX (`Universe`), trait `DmxOutput`, pilote Enttec USB Pro |
| `crates/folie-fixture` | Modèle de fixtures, import OFL, patch, rendu attributs → DMX |
| `crates/folie-show` | Fichier de show `.folie` : lecture, écriture atomique |
| `crates/folie-control` | Console : sélection, groupes, presets, ligne de commande |
| `crates/folie-engine` | Moteur temps réel : thread dédié à ~44 Hz, commandes par canal, état publié |
| `crates/folie-cli` | Binaire `folie` : diagnostic et tests matériels |
| `crates/folie-app` | Binaire `folie-dmx` : interface graphique (egui) |

Principe clé : **le moteur ne dépend jamais de l'interface**. Il reçoit des commandes
et publie un instantané de son état ; une interface lente ne peut pas faire
saccader la sortie DMX.

## Essayer

Rust 1.95 minimum (`rustup update`).

```bash
cargo run -p folie-app                               # interface graphique

cargo run -- ports                                   # lister les ports (FTDI en premier)
cargo run -- test                                    # sortie virtuelle, canaux 1-4
cargo run -- info --port /dev/cu.usbserial-XXXX      # interroger l'USB Pro
cargo run -- test --port /dev/cu.usbserial-XXXX --channels 1-8 --seconds 20
```

Les shows sont des fichiers `.folie` (JSON lisible) qui embarquent les fiches
de fixtures utilisées. Réglages et sauvegarde automatique :
`~/Library/Application Support/Folie-DMX/`.

La bibliothèque de fixtures est dans [library/](library/README.md) (format OFL).
Un autre dossier peut être utilisé avec la variable `FOLIE_LIBRARY`.

Tests : `cargo test`. Pour enregistrer une image de l'interface pendant le test
d'interface : `FOLIE_UI_CAPTURE=<dossier> cargo test -p folie-app`.

Voir [ROADMAP.md](ROADMAP.md) pour la suite.
