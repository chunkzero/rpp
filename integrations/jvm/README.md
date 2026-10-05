# RPP JVM dev client

Java 21+ library connecting server integrations to `rpp dev`. It receives the
current pack on connection and changed packs after rebuilds. Minecraft APIs stay
in caller code. The runnable Minestom example requires Java 25 and Minecraft 26.2.

## Build and use

From the repository root, `mise install` installs both required JDKs and
`mise exec -- just verify-jvm` runs the client tests against RPP and compiles the
Minestom example. For individual Gradle tasks:

```sh
mise exec -- just jvm :test :publishToMavenLocal
```

Every rpp release publishes `com.chunkzero.rpp:rpp-dev-client` at the same version.
Releases are served from `https://maven.chunkzero.com` and nightlies from
`https://maven.chunkzero.com/nightlies`:

```kotlin
repositories { maven("https://maven.chunkzero.com/nightlies") }
dependencies { implementation("com.chunkzero.rpp:rpp-dev-client:0.1.0-nightly.20261004062300.ge282f11816cd") }
```

Use the client version matching your `rpp` binary. Local builds take the Cargo
workspace version, or `RPP_RELEASE_VERSION` when set. Include its runtime
dependencies in your integration's distribution; the library JAR is not a fat JAR.
Jackson is an internal implementation dependency.

```java
var client = new DevClient(
    URI.create("http://127.0.0.1:8080/"), // reachable by the Minecraft server
    URI.create("http://packs.example:8080/"), // reachable by players
    Duration.ofSeconds(2),
    new DevClient.Listener() {
        @Override public void onConnected() {
            logger.info("Connected to RPP");
        }
        @Override public void onUpdate(PackUpdate pack) throws Exception {
            // Offer pack.url(), pack.sha1(), or pack.hashBytes() through your platform.
        }
        @Override public void onFailure(DevClient.FailureKind kind, Exception failure) {
            logger.log(Level.WARNING, "RPP " + kind, failure);
        }
    });
client.start();
// Integration shutdown:
client.close();
```

Callbacks run sequentially on a private virtual thread. Schedule platform work on
the platform's server thread. `latest()` returns the last update accepted by
`onUpdate`, or null before one succeeds; use it when players join.

The client reconnects after EOF, HTTP/network errors, 45 seconds without SSE data,
malformed pack metadata, or a throwing update callback. The constructor supplies
the fixed retry delay; HTTP connection timeout is 10 seconds. A reconnect receives
the latest snapshot, and its SHA-1 suppresses duplicate offers. A throwing
`onUpdate` reports `UPDATE` and retries the latest pack after reconnecting. A
`build_error` reports `BUILD` without disconnecting or offering a pack.

Returning from `onUpdate` acknowledges receipt, not player download success.
Failures in scheduled tasks and player responses must be observed by your adapter;
they cannot throw back into an already-returned callback. Error handlers should
not throw (if they do, the client logs the exception and keeps reconnecting).
Closing interrupts network reads and retry waits; a callback already running may
finish. Create a new client to start again after closing.

## End-to-end Minestom example

Mise exposes JDKs 21 and 25 to Gradle through `JAVA_HOME_21` and `JAVA_HOME_25`.
For a setup without mise, install both JDKs and, if Gradle does not discover them, pass
`-Dorg.gradle.java.installations.paths=/path/to/jdk21,/path/to/jdk25`.
From the repository root, start RPP:

```sh
mise exec -- just rpp -C integrations/jvm/examples/pack dev
```

In another terminal:

```sh
mise exec -- just jvm :examples:minestom:run
```

Join `127.0.0.1:25565` with Minecraft 26.2, allow server resource packs, and open
the pause menu. The example pack changes its heading to “RPP live pack”.
Edit `examples/pack/src/assets/minecraft/lang/en_us.json` under this directory,
changing that heading. RPP rebuilds, publishes a ZIP and SHA-1, and sends metadata
over SSE. The example schedules a pack offer to every online player and prints
each player's pack status. Reopen the pause menu to see the new heading.

The example binds to loopback and uses Minestom's offline mode for local testing.
It offers the current pack on first spawn, uses a stable pack UUID, and replaces
previous packs. Stop both commands with Ctrl-C when finished. The sample format
88.0 matches [Minecraft 26.2](https://feedback.minecraft.net/hc/en-us/articles/46690753273997-Minecraft-Java-Edition-26-2).
The offer and response APIs follow [Minestom's resource-pack guide](https://minestom.net/docs/adventure/resource-packs).

To exercise failures, make `rpp.config.ts` invalid after the server starts: RPP reports a
build error and players keep the previous pack. Restore it without changing pack
content: no duplicate offer. Stop and restart RPP: the integration logs a
connection failure, retries every two seconds, and resumes automatically.
Saving unchanged content does not offer another pack.

For remote players, configure RPP's `[dev].host` and pass server/player base URLs:

```sh
mise exec -- just jvm :examples:minestom:run --args='http://127.0.0.1:8080/ https://packs.example/'
```

The public endpoint must proxy both the download paths and, if used as the event
base, SSE without buffering. Base URLs may include a proxy path prefix. RPP's dev
server has no authentication; use it on a trusted development network.

## Spigot caller integration

Use the same library in your plugin, keeping scheduling and player status handling
in the plugin. This example uses
[Spigot's UUID, URL and raw SHA-1 overload](<https://hub.spigotmc.org/javadocs/spigot/org/bukkit/entity/Player.html#setResourcePack(java.util.UUID,java.lang.String,byte%5B%5D,java.lang.String,boolean)>).
Inside your `JavaPlugin`, keep `client` as a field and initialize it in `onEnable`:

```java
UUID packId = UUID.fromString("bc9527af-fc86-4e82-8810-716797293640");
Consumer<Player> offer = player -> {
    PackUpdate pack = client.latest();
    if (pack != null) {
        try {
            player.setResourcePack(packId, pack.url().toASCIIString(),
                pack.hashBytes(), "Updated development pack", false);
        } catch (RuntimeException failure) {
            getLogger().log(Level.WARNING, "Pack offer failed for " + player.getName(), failure);
        }
    }
};
client = new DevClient(eventBase, playerDownloadBase, Duration.ofSeconds(2),
    new DevClient.Listener() {
        @Override public void onUpdate(PackUpdate pack) {
            Bukkit.getScheduler().runTask(MyPlugin.this, () -> {
                for (Player player : Bukkit.getOnlinePlayers()) {
                    try {
                        player.setResourcePack(packId, pack.url().toASCIIString(),
                            pack.hashBytes(), "Updated development pack", false);
                    } catch (RuntimeException failure) {
                        getLogger().log(Level.WARNING, "Pack offer failed", failure);
                    }
                }
            });
        }
        @Override public void onFailure(DevClient.FailureKind kind, Exception failure) {
            getLogger().log(Level.WARNING, "RPP " + kind, failure);
        }
    });
Bukkit.getPluginManager().registerEvents(new org.bukkit.event.Listener() {
    @EventHandler public void joined(PlayerJoinEvent event) {
        offer.accept(event.getPlayer());
    }
    @EventHandler public void status(PlayerResourcePackStatusEvent event) {
        getLogger().info(event.getPlayer().getName() + ": " + event.getStatus());
    }
}, this);
client.start();
```

Replace `MyPlugin` with your plugin class. Call `client.close()` in `onDisable`.
Player status events expose declined packs and download/application failures.
This is caller code to embed in an existing plugin, not a bundled Spigot plugin.

## Protocol and checks

The wire contract is defined in the dev-server protocol section of [SPEC](../../docs/SPEC.md).
Connection and lag snapshots use named `pack` SSE events; `reload` is reserved
for changed outputs. The JVM client accepts pack metadata from both.
Only the latest ZIP is retained, in memory. Superseded URLs return 404, never new
bytes under an old hash. Rapid rebuilds can overtake a player's download; adapters
should log the resulting player status and offer the newest pack. Dev archives are
unsquashed and independent of release squash/ZIP settings. Archive creation adds
work to each successful dev rebuild.

`just jvm :test` runs local HTTP fixture tests. To also exercise an actual RPP
process, run the verification recipe from the repository root:

```sh
mise exec -- just verify-jvm
```

The integration test uses a temporary pack and isolated RPP user directory, verifies downloaded hashes,
unchanged/failed rebuild suppression, and reconnects across a server restart.
It stops the child server on completion. CI runs this test and compiles the
Minestom example; applying packs in a graphical Minecraft client is a manual check.
