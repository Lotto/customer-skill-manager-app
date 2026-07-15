# Plan — Customer Skill Manager (SkillSync)

## 1. Vision

Application desktop silencieuse (Tauri 2, Rust) qui tourne dans la barre système et maintient à jour les skills métier des clients sous licence. Elle interroge périodiquement le backend, télécharge les skills autorisés par la licence, les installe dans les dossiers cibles (`~/.claude/skills` et dossiers projets configurés), et se met elle-même à jour automatiquement.

Philosophie : **silencieux + logs**. Aucune interaction requise en fonctionnement normal ; tout est traçable dans les logs ; le tray n'expose que l'essentiel (état, dernière sync, quitter).

## 2. Architecture

```
┌─────────────────────────────┐        HTTPS (licence en header)
│  App Tauri (client)         │◄──────────────────────────────┐
│                             │                               │
│  ├─ Tray + fenêtre config   │   ┌───────────────────────────┴──┐
│  ├─ Scheduler (tokio)       │   │  Backend                     │
│  │   ├─ sync skills         │   │  ├─ GET /api/skills/manifest │
│  │   └─ check update        │   │  ├─ GET /api/skills/:id      │
│  ├─ Sync engine (Rust)      │   │  ├─ POST /api/license/verify │
│  ├─ Store licence + config  │   │  └─ GET /api/updates/...     │
│  └─ tauri-plugin-updater    │   └──────────────────────────────┘
└─────────────────────────────┘
```

- **Toute la logique en Rust** (sync, scheduler, licence). Le frontend web n'est qu'une petite fenêtre de configuration/activation, ouverte à la demande depuis le tray.
- **Config locale** : fichier TOML dans le dossier app-data (clé de licence, URL backend, dossiers cibles, intervalle de sync).
- **Logs** : `tracing` + `tracing-appender` (rotation journalière, rétention ~14 jours), niveau configurable.

### Contrat API (côté backend)

- `POST /api/license/verify` → valide la clé, renvoie les droits (skills autorisés, expiration).
- `GET /api/skills/manifest` (auth licence) → liste des skills : `{ id, name, version, hash, path_hint }`.
- `GET /api/skills/:id/:version` (auth licence) → archive du skill (tar.gz ou zip).
- `GET /api/updates/{{target}}/{{arch}}/{{current_version}}` → manifest updater Tauri (ou `204`).

## 3. Phases

### Phase 1 — Fondations (2–3 j)

- [ ] Scaffold Tauri 2 : `npm create tauri-app` (frontend minimal Vite + TS vanilla, pas de framework lourd).
- [ ] Icône tray + menu : état (OK / erreur / sync en cours), "Synchroniser maintenant", "Ouvrir la configuration", "Ouvrir les logs", "Quitter".
- [ ] Pas de fenêtre au démarrage (`"visible": false`), fermeture de fenêtre = masquer, pas quitter.
- [ ] Instance unique (`tauri-plugin-single-instance`) + démarrage automatique avec la session (`tauri-plugin-autostart`).
- [ ] Logging `tracing` → fichier avec rotation + niveau via config.
- [ ] Module config : lecture/écriture TOML dans app-data, valeurs par défaut, rechargement à chaud.

**Livrable** : app qui démarre en tray, logge, se relance avec la session.

### Phase 2 — Moteur de sync (3–5 j)

- [ ] Client HTTP (`reqwest`) avec la clé de licence en header, timeouts, retry avec backoff exponentiel.
- [ ] Récupération du manifest, comparaison avec l'état local (fichier d'état `state.json` : versions + hashes installés).
- [ ] Téléchargement des skills modifiés, vérification du hash, **écriture atomique** (extraction dans un dossier temporaire puis rename) — jamais de skill à moitié écrit.
- [ ] Sync **multi-dossiers** : chaque skill déclare sa cible (global `~/.claude/skills` ou dossiers projets configurés) ; suppression des skills retirés du manifest (uniquement ceux que l'app a installés — marqueur `.csm-managed`).
- [ ] Scheduler `tokio` : sync au démarrage puis toutes les N minutes (configurable, défaut 30 min) ; verrou anti-chevauchement.
- [ ] Gestion offline : échec réseau = log + retry au prochain tick, pas d'alerte.

**Livrable** : skills synchronisés de bout en bout, résilient aux coupures.

### Phase 3 — Licence & UI de configuration (2–3 j)

- [ ] Fenêtre d'activation : saisie de la clé, vérification auprès du backend, stockage (via keyring OS si possible, sinon config chiffrée).
- [ ] Fenêtre de configuration : dossiers cibles, intervalle, niveau de log, état de la licence (expiration, skills couverts).
- [ ] Comportement licence expirée/invalide : les skills restent en place, la sync s'arrête, icône tray en état "attention" + entrée de menu explicite.
- [ ] Premier lancement sans licence : ouvrir la fenêtre d'activation automatiquement (seule exception au silence).

**Livrable** : parcours d'activation complet, app configurable sans toucher aux fichiers.

### Phase 4 — Distribution & auto-update (~1 j + certificats)

- [ ] Génération des clés de signature updater : `npm run tauri signer generate -- -w ~/.tauri/skillsync.key` (clé privée en secret CI, publique dans `tauri.conf.json`).
- [ ] Config `tauri-plugin-updater` : endpoint `https://<backend>/api/updates/{{target}}/{{arch}}/{{current_version}}`, servi derrière la même auth que les skills (permet aussi le déploiement progressif).
- [ ] Check d'update intégré au scheduler existant, mode **semi-automatique** : téléchargement en arrière-plan, puis entrée tray "Redémarrer pour mettre à jour" — jamais de restart automatique pendant une sync.
- [ ] CI (GitHub Actions + `tauri-action`) : build Windows (NSIS), macOS (`.app.tar.gz`), Linux (AppImage uniquement), signature, publication des artefacts + manifest.
- [ ] Selon les OS des clients : certificat code-signing Windows (sinon avertissement SmartScreen) ; compte Apple Developer + notarisation si clients macOS.

**Livrable** : release v1.0 installable, qui se met à jour toute seule.

### Phase 5 — Durcissement & observabilité (2–3 j, itératif)

- [ ] Tests : unitaires sur le moteur de sync (diff manifest/état, atomicité), test d'intégration contre un backend mock.
- [ ] Remontée d'état optionnelle au backend (heartbeat : version, dernière sync, erreurs) → visibilité sur le parc client.
- [ ] Entrée tray "Signaler un problème" : zippe les logs récents pour envoi.
- [ ] Documentation : guide d'installation client (1 page), runbook de release.

## 4. Décisions à trancher

| Question | Impact | Recommandation |
|---|---|---|
| OS des clients ? | Budget certificats (Windows ~300 €/an, Apple 99 $/an) et priorité de packaging | Commencer Windows-only si le parc le permet |
| Backend : existant ou à créer ? | Ajoute une phase 0 (API + stockage skills + licences) si à créer | Si à créer : GitHub Releases pour l'updater en MVP, backend uniquement pour skills + licences |
| Stockage de la clé de licence | Keyring OS = plus sûr mais code par plateforme | Keyring via `keyring-rs`, fallback fichier |

## 5. Risques

- **Écriture pendant qu'un skill est en cours d'utilisation** : l'écriture atomique (rename) limite le risque ; ne jamais supprimer-puis-écrire.
- **SmartScreen/Gatekeeper sans signature** : acceptable en MVP interne, bloquant pour de la vente — à budgéter tôt.
- **Suppression de fichiers utilisateur** : ne supprimer que ce qui porte le marqueur `.csm-managed`.

**Estimation totale : ~10–15 jours** de dev (hors backend s'il est à créer, hors délais d'obtention des certificats).
