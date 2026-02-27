# ConditionAlarm — Event-Driven Alarm System

An Android alarm app that triggers not on time, but on **conditions** — real-world events pulled from APIs. "Remind me when it stops raining", "Alert me when headlines mention a product launch", "Notify me when temperature drops below 10°C."

---

## Table of Contents

- [Overview](#overview)
- [Tech Stack](#tech-stack)
- [Project Structure](#project-structure)
- [Getting Started](#getting-started)
- [Documentation](#documentation)
- [MVP Scope](#mvp-scope)

---

## Overview

The user creates a **Reminder** with one or more **Conditions**. Conditions are built from API data using a field, an operator, and a value (e.g. `weather.main == "Clear"`). Conditions can be composed with AND / OR logic into a tree. A background service polls the APIs on a schedule and fires a notification — TTS voice output or phone ringing — when all conditions evaluate to true.

Users can build conditions **manually** by selecting an API from a dropdown, filling in fields via that API's own UI, or via **AI** by describing the condition in natural language.

---

## Tech Stack

| Layer | Technology |
|---|---|
| UI | Jetpack Compose |
| Async | Coroutines + Flow |
| Local DB | Room |
| JSON | Moshi (with polymorphic adapters) |
| Preferences | DataStore |
| HTTP | Ktor Client |
| IDE | Android Studio |
| Language | Kotlin |
| DI | Hilt |

---

## Project Structure

```
app/
├── src/main/
│   ├── java/com/conditionalarm/
│   │   ├── data/
│   │   │   ├── db/               # Room database, DAOs, entities
│   │   │   ├── datastore/        # DataStore preferences
│   │   │   └── repository/       # ReminderRepository
│   │   ├── domain/
│   │   │   ├── model/            # Reminder, Condition, LeafCondition, CompositeCondition
│   │   │   └── evaluation/       # Condition tree evaluation logic
│   │   ├── api/
│   │   │   ├── core/             # ApiManager (abstract), ApiRegistry, ApiField
│   │   │   └── managers/         # Concrete ApiManager implementations (e.g. OpenWeatherApiManager)
│   │   ├── ai/
│   │   │   └── AIConditionBuilder.kt
│   │   ├── service/
│   │   │   └── BackgroundEvaluationService.kt
│   │   ├── notification/
│   │   │   └── NotificationService.kt
│   │   ├── ui/
│   │   │   ├── menu/             # AlarmMenuScreen
│   │   │   ├── options/          # AlarmOptionsScreen
│   │   │   ├── condition/        # ConditionTile, condition builder UI
│   │   │   └── theme/            # Compose theme
│   │   └── di/                   # Hilt modules
│   └── assets/
│       └── apis.json             # ApiRegistry config file
```

---

## Getting Started

### Prerequisites

- Android Studio Hedgehog or newer
- Android SDK 26+
- An Anthropic API key (for AI condition building)
- API keys for any concrete ApiManagers you enable

### Setup

1. Clone the repository.

2. Create `local.properties` in the project root and add your keys:

```
ANTHROPIC_API_KEY=sk-ant-...
OPENWEATHER_API_KEY=...
```

3. Sync Gradle and run on a device or emulator (API 26+).

4. The app reads `assets/apis.json` at startup to populate the `ApiRegistry`. Add or remove API entries there to control which APIs are available.

---

## Documentation

| File | Description |
|---|---|
| [ARCHITECTURE.md](./ARCHITECTURE.md) | System design, layers, key decisions |
| [DATA_MODEL.md](./DATA_MODEL.md) | All domain models, Room schema, JSON schemas |
| [API_LAYER.md](./API_LAYER.md) | How ApiManager works, how to add a new one |
| [AI_INTEGRATION.md](./AI_INTEGRATION.md) | AI condition builder, tool definitions, flow |
| [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md) | Ordered build plan for the MVP |

---

## MVP Scope

The MVP includes:

- Reminder list screen (create, view, delete)
- Reminder options screen (title, description, notification method, triggerOnce)
- Manual condition builder with at least one concrete ApiManager implemented
- AND / OR composite condition logic
- Background polling service that evaluates conditions
- TTS notification and phone ringing notification
- AI condition builder via Anthropic API tool calling
- Persistence via Room + Moshi
- ApiRegistry loaded from `apis.json`

**Not in MVP:**
- Temporal / duration conditions ("for 3 days", "since yesterday")
- Remote update of apis.json
- Multiple concrete ApiManagers (add incrementally after MVP)
