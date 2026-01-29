# Demo Resource Pack - RPP 2.0

This is a demonstration resource pack showing the new RPP 2.0 architecture in action.

## Structure

```
demo_pack/
├── rpp.jsonc                   # Pack configuration
├── src/                        # Resource pack contents
│   ├── pack.mcmeta            # Pack metadata
│   └── assets/
│       └── minecraft/
│           └── textures/
│               ├── block/
│               │   └── dirt.png.mcmeta    # Animation metadata
│               └── item/
│                   └── diamond.json        # Item model
├── plugins/                    # RPP Lua plugins
│   ├── json_minify.lua         # Minifies JSON files
│   ├── hash_renamer.lua        # Renames files with content hash
│   └── mcmeta_validator.lua    # Validates .mcmeta files
└── .rpp/
    ├── build/                  # Final build output
    │   └── pack.zip           # Zip archive
    └── cache/                  # Build cache
        └── build.cache
```

## Plugins

### 1. mcmeta_validator.lua (Priority: 10)
- Validates animation metadata
- Checks frametime ranges
- Warns about unusual configurations
- Runs first to catch issues early

### 2. json_minify.lua (Priority: 50)
- Removes whitespace from JSON files
- Reduces file size
- Reports compression ratio
- Runs on all `*.json` files

### 3. hash_renamer.lua (Priority: 200)
- Renames PNG files based on content hash
- Enables aggressive browser caching
- Format: `filename.<hash>.png`
- Runs last to hash final content

## Building

Build the pack from the demo_pack directory:
```bash
cd examples/demo_pack
cargo run -p rpp-cli -- build
```

The built pack will be in `.rpp/build/` with a `pack.zip` archive.

Build with clean:
```bash
cargo run -p rpp-cli -- build --clean
```

Build with custom output:
```bash
cargo run -p rpp-cli -- build --output /tmp/my_pack
```

Build with custom source:
```bash
cargo run -p rpp-cli -- build --source custom_src
```

## Configuration

The `rpp.jsonc` file contains pack configuration:
```jsonc
{
  "sourceDir": "src",           // Source directory
  "devServer": {
    "host": "127.0.0.1",
    "port": 8080,
    "hotReload": true
  },
  "pluginRepositories": []
}
```

CLI arguments override configuration values.

## Development Server

Start the dev server from the demo_pack directory:
```bash
cd examples/demo_pack
cargo run -p rpp-cli -- serve
```

Then open http://localhost:8080 in your browser. The server will automatically rebuild when you modify files in `src/`.

## Features Demonstrated

1. **Processor Chaining**: Files go through multiple processors in priority order
2. **Lua Plugin API**: Full access to JSON, hash, and logging APIs
3. **Path Transformation**: hash_renamer changes output paths
4. **Incremental Builds**: Cache avoids reprocessing unchanged files
5. **Parallel Processing**: Worker pool processes files concurrently
6. **Hot Reload**: Dev server watches for changes and rebuilds

## Expected Output

When you build this pack, you should see:
- JSON files minified (smaller size)
- PNG files renamed with content hashes
- Animation metadata validated with logged info
- Files written to `.rpp/build/`
- Zip archive created at `.rpp/build/pack.zip`
- Build cache at `.rpp/cache/build.cache` for future incremental builds

Try modifying a file in `src/` and rebuilding to see the cache in action!
