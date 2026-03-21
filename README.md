# waterpi-sprinkler

Contrôleur d'arrosage enterré pour Raspberry Pi + Home Assistant.

## Architecture

```
┌──────────────────────┐          ┌──────────────────────────────┐
│       pi5 (HA)       │          │      waterpi (Raspberry Pi)  │
│                      │          │                              │
│  custom_components/  │  REST    │  waterpi-sprinkler (Rust)    │
│  waterpi_sprinkler/  │◄────────►│  ├─ axum REST API :8090      │
│  └─ valve entities   │  poll +  │  ├─ rppal GPIO control       │
│                      │  command │  ├─ mutex (1 vanne à la fois) │
│                      │◄─────── │  └─ safety timeout (30min)   │
│  event bus           │  push   │                              │
│                      │  events  │  GPIO 5  → Relais → Vanne 1 │
│                      │          │  GPIO 6  → Relais → Vanne 2 │
│                      │          │  GPIO 13 → Relais → Vanne 3 │
│                      │          │  GPIO 19 → Relais → Vanne 4 │
└──────────────────────┘          └──────────────────────────────┘
```

- **Polling** : HA interroge le daemon toutes les 10s pour l'état des vannes
- **Push** : le daemon fire un event HA (`waterpi_sprinkler_update`) à chaque changement d'état
- **Commandes** : HA appelle le daemon en REST pour ouvrir/fermer les vannes

## Setup — Daemon (waterpi)

### Prérequis

```bash
# Sur waterpi, installer libgpiod (filet de sécurité systemd)
sudo apt install gpiod

# Installer la toolchain Rust si pas déjà présente
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Compilation

```bash
cd daemon
cargo build --release
```

### Installation

```bash
sudo mkdir -p /etc/waterpi-sprinkler
sudo cp target/release/waterpi-sprinkler /usr/local/bin/
sudo cp config.example.toml /etc/waterpi-sprinkler/config.toml

# Éditer la config (token HA, vérifier les GPIO)
sudo nano /etc/waterpi-sprinkler/config.toml

# Installer et démarrer le service
sudo cp waterpi-sprinkler.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now waterpi-sprinkler

# Vérifier
sudo systemctl status waterpi-sprinkler
curl http://localhost:8090/api/zones
```

### Créer un Long-Lived Token HA

1. Dans HA → Profil utilisateur → Jetons d'accès longue durée
2. Créer un token, le copier dans `config.toml` → `[ha].token`

## Setup — Custom Component HA (pi5)

### Installation

Copier le dossier `custom_components/waterpi_sprinkler/` dans le
répertoire `custom_components/` de votre installation HA :

```bash
cp -r ha_component/custom_components/waterpi_sprinkler \
      /config/custom_components/
```

### Configuration

Dans `configuration.yaml` :

```yaml
waterpi_sprinkler:
  host: waterpi      # hostname ou IP du Raspberry Pi
  port: 8090
```

Redémarrer Home Assistant.

Quatre entités `valve.arrosage_zone_*` apparaissent, regroupées sous
un device "WaterPi Sprinkler".

## API REST du daemon

| Méthode | Endpoint                    | Description              |
|---------|-----------------------------|--------------------------|
| GET     | `/api/health`               | Health check             |
| GET     | `/api/zones`                | Liste toutes les zones   |
| GET     | `/api/zones/{id}`           | État d'une zone          |
| POST    | `/api/zones/{id}/open`      | Ouvrir une vanne         |
| POST    | `/api/zones/{id}/close`     | Fermer une vanne         |
| POST    | `/api/zones/close-all`      | Fermer toutes les vannes |

### Exemple de réponse

```json
{
  "id": "arrosage1",
  "name": "Arrosage Zone 1",
  "gpio": 5,
  "is_open": true,
  "opened_at": "2026-03-21T14:30:00+00:00",
  "open_duration_secs": 120,
  "max_duration_secs": 1800
}
```

## Sécurités

1. **Durée max** : chaque zone se ferme automatiquement après 30 min (configurable)
2. **Mutex** : une seule vanne ouverte à la fois (configurable)
3. **Graceful shutdown** : toutes les vannes sont fermées sur SIGTERM/SIGINT
4. **Filet systemd** : `ExecStopPost` force les GPIO HIGH si le daemon crash
5. **Startup safe** : toutes les vannes démarrent fermées

## Cross-compilation (optionnel)

Si tu préfères compiler sur un autre poste qu'un Pi :

```bash
# Ajouter la target ARM
rustup target add armv7-unknown-linux-gnueabihf  # Pi 3/4
# ou
rustup target add aarch64-unknown-linux-gnu       # Pi 4/5 en 64-bit

# Compiler
cargo build --release --target aarch64-unknown-linux-gnu

# Copier le binaire
scp target/aarch64-unknown-linux-gnu/release/waterpi-sprinkler pi@waterpi:/tmp/
```

Tu auras besoin du linker cross-compilation (`aarch64-linux-gnu-gcc`).
