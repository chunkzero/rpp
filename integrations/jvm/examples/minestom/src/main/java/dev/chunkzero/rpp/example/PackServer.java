package dev.chunkzero.rpp.example;

import java.net.URI;
import java.time.Duration;
import java.util.UUID;

import dev.chunkzero.rpp.DevClient;
import dev.chunkzero.rpp.PackUpdate;
import net.kyori.adventure.resource.ResourcePackInfo;
import net.kyori.adventure.resource.ResourcePackRequest;
import net.minestom.server.MinecraftServer;
import net.minestom.server.coordinate.Pos;
import net.minestom.server.entity.Player;
import net.minestom.server.event.player.AsyncPlayerConfigurationEvent;
import net.minestom.server.event.player.PlayerSpawnEvent;
import net.minestom.server.instance.block.Block;

/** Local development server; accepts offline connections on loopback only. */
public final class PackServer {
    private static final UUID PACK_ID = UUID.fromString("bc9527af-fc86-4e82-8810-716797293640");

    public static void main(String[] args) {
        URI events = URI.create(args.length > 0 ? args[0] : "http://127.0.0.1:8080/");
        URI downloads = URI.create(args.length > 1 ? args[1] : events.toString());
        var server = MinecraftServer.init();
        var instance = MinecraftServer.getInstanceManager().createInstanceContainer();
        instance.setGenerator(unit -> unit.modifier().fillHeight(0, 40, Block.STONE));
        MinecraftServer.getGlobalEventHandler().addListener(AsyncPlayerConfigurationEvent.class, event -> {
            event.setSpawningInstance(instance);
            event.getPlayer().setRespawnPoint(new Pos(0, 42, 0));
        });
        var client = new DevClient(events, downloads, Duration.ofSeconds(2), new DevClient.Listener() {
            @Override
            public void onConnected() { System.out.println("Connected to RPP"); }

            @Override
            public void onUpdate(PackUpdate pack) {
                MinecraftServer.getSchedulerManager().scheduleNextTick(() ->
                    MinecraftServer.getConnectionManager().getOnlinePlayers().forEach(player -> offer(player, pack)));
            }

            @Override
            public void onFailure(DevClient.FailureKind kind, Exception failure) {
                System.err.println("RPP " + kind + ": " + failure.getMessage());
            }
        });
        MinecraftServer.getGlobalEventHandler().addListener(PlayerSpawnEvent.class, event -> {
            PackUpdate pack = client.latest();
            if (event.isFirstSpawn() && pack != null) offer(event.getPlayer(), pack);
        });
        Runtime.getRuntime().addShutdownHook(new Thread(client::close));
        client.start();
        server.start("127.0.0.1", 25565);
    }

    private static void offer(Player player, PackUpdate pack) {
        try {
            player.sendResourcePacks(ResourcePackRequest.resourcePackRequest()
                    .packs(ResourcePackInfo.resourcePackInfo(PACK_ID, pack.url(), pack.sha1()))
                    .replace(true)
                    .required(false)
                    .callback((id, status, audience) ->
                            System.out.println("Pack " + pack.sha1() + " for " + player.getUsername() + ": " + status))
                    .build());
        } catch (RuntimeException failure) {
            System.err.println("Pack offer failed for " + player.getUsername() + ": " + failure.getMessage());
        }
    }
}
