# ApexStore Frontend

Angular 17 dashboard for the [ApexStore](https://github.com/ElioNeto/ApexStore) REST API.

## Tech Stack

- **Angular 17** (standalone components)
- **Signals** (`signal`, `computed`, `input`)
- **New template syntax** (`@if`, `@for`)
- **No external UI libraries** — pure SCSS

## Pages

| Page | Route | Description |
|------|-------|-------------|
| Dashboard | `/dashboard` | PUT/GET ops + operation history + live stats |
| Key Explorer | `/keys` | Table view of fetched keys with refetch |
| Statistics | `/stats` | Full telemetry from `/stats/all` |

## Setup

```bash
cd frontend
npm install
npm start
# Open http://localhost:4200
```

## API Configuration

Edit `src/environments/environment.ts` to change the API URL:

```ts
export const environment = {
  production: false,
  apiUrl: 'http://localhost:8080', // change if needed
};
```

## Backend (ApexStore)

Start the API server:

```bash
# Docker
docker-compose up -d

# Or locally
cargo run --release -- --server
```

The frontend calls:
- `POST /keys` — insert/update a key
- `GET /keys/{key}` — retrieve a value
- `GET /stats/all` — full telemetry
