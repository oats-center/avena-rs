# Avena webapp

SvelteKit app for watching Avena edge nodes live, editing their LabJack
configuration, and downloading archived data as CSV. It runs in the browser and
talks only to central NATS over WebSocket.

```bash
pnpm install
pnpm dev            # http://localhost:5173
pnpm build          # Node server in build/, run with `node build`
```

Connect with the central WebSocket address (`ws://nats1.oats:8080`) and a
`.creds` file.

Documentation, including how the app is put together, its components and how
to deploy it, is at <https://oats-center.github.io/avena-rs/> (source in
`../docs`). The TypeScript API reference is built with
`pnpm exec typedoc --options typedoc.json`.
