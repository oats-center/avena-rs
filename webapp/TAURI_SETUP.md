# Tauri Desktop App Setup for Avena Dashboard

This guide explains how to build and run your Svelte webapp as a cross-platform desktop application using Tauri.

## Project Structure

```
webapp/
├── src-tauri/                 # Tauri backend (Rust)
│   ├── src/
│   │   ├── main.rs           # Entry point
│   │   └── lib.rs            # Main application logic
│   ├── Cargo.toml            # Rust dependencies
│   ├── tauri.conf.json       # Tauri configuration
│   ├── capabilities/         # Security permissions
│   └── icons/                # App icons
├── src/                      # Your Svelte frontend
├── build/                    # Built static files (for Tauri)
└── package.json              # Updated with Tauri scripts
```

## Available Commands

### Development
```bash
# Start Tauri development mode (hot reload)
pnpm tauri:dev

# Start regular web development
pnpm dev
```

### Building
```bash
# Build for production (all platforms)
pnpm tauri:build

# Build debug version
pnpm tauri:build:debug

# Build web version only
pnpm build
```

### Platform-Specific Builds
```bash
# Build for specific platform
pnpm tauri build --target x86_64-pc-windows-msvc    # Windows
pnpm tauri build --target x86_64-unknown-linux-gnu  # Linux
pnpm tauri build --target x86_64-apple-darwin       # macOS Intel
pnpm tauri build --target aarch64-apple-darwin      # macOS Apple Silicon
```

## Configuration Details

### Tauri Configuration (`src-tauri/tauri.conf.json`)
- **App Name**: "Avena Dashboard"
- **Window Size**: 1200x800 (minimum 800x600)
- **Dev URL**: http://localhost:5173 (Svelte dev server)
- **Build Output**: `../build` (SvelteKit static build)

### Permissions (`src-tauri/capabilities/default.json`)
Configured for your LabJack dashboard needs:
- File system access (read/write files, directories)
- HTTP requests (for NATS connectivity)
- Shell operations (opening external links)

### SvelteKit Configuration
- **Adapter**: `@sveltejs/adapter-static` for Tauri compatibility
- **Output**: Static files in `build/` directory
- **Fallback**: `index.html` for SPA routing

## Development Workflow

1. **Start Development**:
   ```bash
   pnpm tauri:dev
   ```
   This will:
   - Start the Svelte dev server on port 5173
   - Launch the Tauri desktop app
   - Enable hot reload for both frontend and backend

2. **Make Changes**:
   - Edit Svelte files in `src/` - changes appear instantly
   - Edit Rust files in `src-tauri/src/` - app restarts automatically

3. **Test Features**:
   - NATS connectivity works in desktop app
   - File system operations (credential uploads)
   - Real-time data visualization

## Building for Distribution

### macOS
```bash
pnpm tauri:build
# Creates: src-tauri/target/release/bundle/macos/Avena Dashboard.app
# Also creates: .dmg installer
```

### Windows
```bash
pnpm tauri build --target x86_64-pc-windows-msvc
# Creates: src-tauri/target/release/bundle/msi/Avena Dashboard_0.1.0_x64_en-US.msi
```

### Linux
```bash
pnpm tauri build --target x86_64-unknown-linux-gnu
# Creates: src-tauri/target/release/bundle/deb/Avena Dashboard_0.1.0_amd64.deb
```

## Customization

### App Icon
Replace files in `src-tauri/icons/`:
- `icon.png` (512x512)
- `icon.icns` (macOS)
- `icon.ico` (Windows)
- Various PNG sizes for different contexts

### Window Properties
Edit `src-tauri/tauri.conf.json`:
```json
{
  "app": {
    "windows": [{
      "title": "Avena Dashboard",
      "width": 1200,
      "height": 800,
      "minWidth": 800,
      "minHeight": 600,
      "resizable": true,
      "fullscreen": false
    }]
  }
}
```


## Resources

- [Tauri Documentation](https://tauri.app/)
- [Tauri API Reference](https://tauri.app/v1/api/js/)
- [SvelteKit Adapter Static](https://kit.svelte.dev/docs/adapter-static)
- [Cross-Platform Build Guide](https://tauri.app/v1/guides/building/cross-platform)
