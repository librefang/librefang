# Premiers pas avec LibreFang

Ce guide vous accompagne dans l'installation de LibreFang, la configuration de votre premier fournisseur LLM, le lancement d'un agent et la conversation avec celui-ci.

Site web du projet : [https://librefang.ai/](https://librefang.ai/)

## Table des matières

- [Installation](#installation)
- [Configuration](#configuration)
- [Lancer votre premier agent](#lancer-votre-premier-agent)
- [Discuter avec un agent](#discuter-avec-un-agent)
- [Démarrer le daemon](#démarrer-le-daemon)
- [Utiliser l'interface WebChat](#utiliser-linterface-webchat)
- [Étapes suivantes](#étapes-suivantes)

---

## Installation

### Option 1 : Cargo Install (toutes plateformes)

LibreFang ne publie pas encore de GitHub Releases. La méthode d'installation recommandée actuellement est :

```bash
cargo install --git https://github.com/librefang/librefang librefang-cli
```

Ou compiler depuis les sources :

```bash
git clone https://github.com/librefang/librefang.git
cd librefang
cargo install --path crates/librefang-cli
```

### Option 2 : Installateur Shell (Linux / macOS / WSL)

```bash
curl -fsSL https://librefang.ai/install.sh | sh
```

Le script installe le binaire CLI dans `~/.librefang/bin/` et ajoute le répertoire à votre PATH.

### Option 3 : Installateur PowerShell (Windows, pour les futures versions)

```powershell
irm https://librefang.ai/install.ps1 | iex
```

Utilisez cette méthode une fois que LibreFang commencera à publier des GitHub Releases. Le script vérifie les sommes de contrôle SHA256 et ajoute le CLI à votre PATH utilisateur.

### Option 4 : Docker

```bash
docker pull ghcr.io/librefang/librefang:latest

docker run -d \
  --name librefang \
  -p 4545:4545 \
  -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY \
  -v librefang-data:/data \
  ghcr.io/librefang/librefang:latest
```

Ou utilisez Docker Compose :

```bash
git clone https://github.com/librefang/librefang.git
cd librefang
# Définissez vos clés API dans l'environnement ou un fichier .env
docker compose up -d
```

### Vérifier l'installation

```bash
librefang --version
```

---

## Configuration

### Initialisation

Exécutez la commande init pour créer le répertoire `~/.librefang/` et un fichier de configuration par défaut :

```bash
librefang init
```

Cela crée :

```
~/.librefang/
  config.toml    # Configuration principale
  data/          # Base de données et données d'exécution
  agents/        # Manifestes d'agents (optionnel)
```

### Configurer une clé API

LibreFang nécessite au moins une clé API de fournisseur LLM. Définissez-la comme variable d'environnement :

```bash
# Anthropic (Claude)
export ANTHROPIC_API_KEY=sk-ant-...

# Ou OpenAI
export OPENAI_API_KEY=sk-...

# Ou Groq (niveau gratuit disponible)
export GROQ_API_KEY=gsk_...
```

Ajoutez l'export à votre profil shell (`~/.bashrc`, `~/.zshrc`, etc.) pour le rendre persistant.

### Modifier la configuration

La configuration par défaut utilise Anthropic. Pour changer de fournisseur, modifiez `~/.librefang/config.toml` :

```toml
[default_model]
provider = "groq"                      # anthropic, openai, groq, ollama, etc.
model = "llama-3.3-70b-versatile"      # Identifiant du modèle pour le fournisseur
api_key_env = "GROQ_API_KEY"           # Variable d'env contenant la clé API

[memory]
decay_rate = 0.05                      # Taux de décroissance de la confiance mémoire

[network]
listen_addr = "127.0.0.1:4545"        # Adresse d'écoute OFP
```

### Vérifier votre configuration

```bash
librefang doctor
```

Cela vérifie que votre configuration existe, que les clés API sont définies et que la chaîne d'outils est disponible.

---

## Lancer votre premier agent

### Utiliser un modèle intégré

LibreFang est livré avec 30 modèles d'agents. Lancez l'agent hello-world :

```bash
librefang agent spawn agents/hello-world/agent.toml
```

Sortie :

```
Agent spawned successfully!
  ID:   a1b2c3d4-e5f6-...
  Name: hello-world
```

### Utiliser un manifeste personnalisé

Créez votre propre `my-agent.toml` :

```toml
name = "my-assistant"
version = "0.4.0"
description = "A helpful assistant"
author = "you"
module = "builtin:chat"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"

[capabilities]
tools = ["file_read", "file_list", "web_fetch"]
memory_read = ["*"]
memory_write = ["self.*"]
```

Puis lancez-le :

```bash
librefang agent spawn my-agent.toml
```

### Lister les agents en cours d'exécution

```bash
librefang agent list
```

Sortie :

```
ID                                     NAME             STATE      PROVIDER     MODEL
-----------------------------------------------------------------------------------------------
a1b2c3d4-e5f6-...                     hello-world      Running    groq         llama-3.3-70b-versatile
```

---

## Discuter avec un agent

Démarrez une session de chat interactive en utilisant l'identifiant de l'agent :

```bash
librefang agent chat a1b2c3d4-e5f6-...
```

Ou utilisez la commande de chat rapide (sélectionne le premier agent disponible) :

```bash
librefang chat
```

Ou spécifiez un agent par son nom :

```bash
librefang chat hello-world
```

Exemple de session :

```
Chat session started (daemon mode). Type 'exit' or Ctrl+C to quit.

you> Hello! What can you do?

agent> I'm the hello-world agent running on LibreFang. I can:
- Read files from the filesystem
- List directory contents
- Fetch web pages

Try asking me to read a file or look up something on the web!

  [tokens: 142 in / 87 out | iterations: 1]

you> List the files in the current directory

agent> Here are the files in the current directory:
- Cargo.toml
- Cargo.lock
- README.md
- agents/
- crates/
- docs/
...

you> exit
Chat session ended.
```

---

## Démarrer le daemon

Pour des agents persistants, un accès multi-utilisateurs et l'interface WebChat, démarrez le daemon :

```bash
librefang start
```

Sortie :

```
[ok] Daemon started in background

API:        http://127.0.0.1:4545
Dashboard:  http://127.0.0.1:4545/
hint: Use `librefang stop` to stop the daemon
```

Le daemon fournit :

- **API REST** à `http://127.0.0.1:4545/api/`
- **WebSocket** à `ws://127.0.0.1:4545/api/agents/{id}/ws`
- **Interface WebChat** à `http://127.0.0.1:4545/`
- **Réseau OFP** sur le port 4545

### Vérifier le statut

```bash
librefang status
```

### Arrêter le daemon

Appuyez sur `Ctrl+C` dans le terminal exécutant le daemon, ou :

```bash
curl -X POST http://127.0.0.1:4545/api/shutdown
```

---

## Utiliser l'interface WebChat

Avec le daemon en cours d'exécution, ouvrez votre navigateur à l'adresse :

```
http://127.0.0.1:4545/
```

L'interface WebChat intégrée vous permet de :

- Voir tous les agents en cours d'exécution
- Discuter avec n'importe quel agent en temps réel (via WebSocket)
- Voir les réponses en streaming au fur et à mesure de leur génération
- Consulter l'utilisation des tokens par message

---

## Étapes suivantes

Maintenant que LibreFang est en cours d'exécution :

- **Explorez les modèles d'agents** : Parcourez le répertoire `agents/` pour découvrir 30 agents pré-construits (codeur, chercheur, rédacteur, ops, analyste, auditeur de sécurité, et plus).
- **Créez des agents personnalisés** : Rédigez vos propres manifestes `agent.toml`. Consultez le [Guide d'architecture](architecture.md) pour les détails sur les capacités et la planification.
- **Configurez des canaux** : Connectez l'une des 40 plateformes de messagerie (Telegram, Discord, Slack, WhatsApp, LINE, Mastodon, et 34 autres). Voir [Adaptateurs de canaux](channel-adapters.md).
- **Utilisez les compétences intégrées** : 60 compétences expertes sont pré-installées (GitHub, Docker, Kubernetes, audit de sécurité, ingénierie de prompts, etc.). Voir [Développement de compétences](skill-development.md).
- **Créez des compétences personnalisées** : Étendez les agents avec Python, WASM ou des compétences basées uniquement sur des prompts. Voir [Développement de compétences](skill-development.md).
- **Utilisez l'API** : 76 endpoints REST/WS/SSE, incluant un `/v1/chat/completions` compatible OpenAI. Voir [Référence API](api-reference.md).
- **Changez de fournisseur LLM** : 20 fournisseurs supportés (Anthropic, OpenAI, Gemini, Groq, DeepSeek, xAI, Ollama, et plus). Surcharge du modèle par agent possible.
- **Configurez des workflows** : Chaînez plusieurs agents ensemble. Utilisez `librefang workflow create` avec une définition de workflow TOML.
- **Utilisez MCP** : Connectez-vous à des outils externes via le Model Context Protocol. Configurez dans `config.toml` sous `[[mcp_servers]]`.
- **Migrez depuis OpenClaw** : Exécutez `librefang migrate --from openclaw`. Voir [MIGRATION.md](../docs/MIGRATION.md).
- **Application de bureau** : Exécutez `cargo tauri dev` pour une expérience de bureau native avec icône dans la barre système.
- **Lancez les diagnostics** : `librefang doctor` vérifie l'ensemble de votre configuration.

### Référence des commandes utiles

```bash
librefang init                          # Initialiser ~/.librefang/
librefang start                         # Démarrer le daemon
librefang status                        # Vérifier le statut du daemon
librefang doctor                        # Lancer les vérifications diagnostiques

librefang agent spawn <manifest.toml>   # Lancer un agent
librefang agent list                    # Lister tous les agents
librefang agent chat <id>               # Discuter avec un agent
librefang agent kill <id>               # Arrêter un agent

librefang workflow list                 # Lister les workflows
librefang workflow create <file.json>   # Créer un workflow
librefang workflow run <id> <input>     # Exécuter un workflow

librefang trigger list                  # Lister les déclencheurs d'événements
librefang trigger create <args>         # Créer un déclencheur
librefang trigger delete <id>           # Supprimer un déclencheur

librefang skill install <source>        # Installer une compétence
librefang skill list                    # Lister les compétences installées
librefang skill search <query>          # Rechercher sur FangHub
librefang skill test [path]             # Valider une compétence locale
librefang skill publish [path]          # Empaqueter/publier un bundle de compétences
librefang skill create                  # Créer le squelette d'une nouvelle compétence

librefang channel list                  # Lister le statut des canaux
librefang channel setup <channel>       # Assistant de configuration interactif

librefang config show                   # Afficher la configuration actuelle
librefang config edit                   # Ouvrir la configuration dans l'éditeur

librefang chat [agent]                  # Chat rapide (alias)
librefang migrate --from openclaw       # Migrer depuis OpenClaw
librefang mcp                           # Démarrer le serveur MCP (stdio)
```
