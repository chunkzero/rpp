package dev.chunkzero.rpp;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.condition.EnabledIfEnvironmentVariable;
import org.junit.jupiter.api.io.TempDir;

import java.net.ServerSocket;
import java.net.URI;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.time.Duration;
import java.util.HexFormat;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;

@EnabledIfEnvironmentVariable(named = "RPP_BIN", matches = ".+")
class RppIntegrationTest {
    @TempDir Path project;

    @Test
    void edits_downloads_failures_and_restart_use_the_real_dev_server() throws Exception {
        int port;
        try (var socket = new ServerSocket(0)) {
            port = socket.getLocalPort();
        }
        var config = project.resolve("rpp.config.ts");
        String validConfig =
                "export default { pack: { name: 'jvm-test', format: 34 }, dev: { port: "
                        + port
                        + " } };\n";
        Files.writeString(config, validConfig);
        Files.createDirectory(project.resolve("src"));
        Path source = project.resolve("src/marker.json");
        Files.writeString(source, "{}");
        var updates = new LinkedBlockingQueue<PackUpdate>();
        var buildFailures = new LinkedBlockingQueue<Exception>();
        var connections = new LinkedBlockingQueue<Boolean>();
        URI base = URI.create("http://127.0.0.1:" + port);
        Process server = launch();
        try (var client =
                new DevClient(
                        base,
                        base,
                        Duration.ofMillis(100),
                        new DevClient.Listener() {
                            @Override
                            public void onConnected() {
                                connections.add(true);
                            }

                            @Override
                            public void onUpdate(PackUpdate pack) {
                                updates.add(pack);
                            }

                            @Override
                            public void onFailure(DevClient.FailureKind kind, Exception failure) {
                                if (kind == DevClient.FailureKind.BUILD) buildFailures.add(failure);
                            }
                        })) {
            client.start();
            var first = updates.poll(20, TimeUnit.SECONDS);
            assertNotNull(first);
            assertNotNull(connections.poll(5, TimeUnit.SECONDS));
            verifyDownload(first);
            Files.writeString(source, "{\"changed\":true}");
            var second = updates.poll(10, TimeUnit.SECONDS);
            assertNotNull(second);
            assertNotEquals(first.sha1(), second.sha1());
            verifyDownload(second);
            Files.writeString(source, "{\"changed\":true}");
            assertNull(updates.poll(1, TimeUnit.SECONDS));

            Files.writeString(config, "export default {");
            assertNotNull(buildFailures.poll(10, TimeUnit.SECONDS));
            assertTrue(updates.isEmpty());
            verifyDownload(second);

            server.destroy();
            assertTrue(server.waitFor(5, TimeUnit.SECONDS));
            Files.writeString(config, validConfig);
            server = launch();
            assertNotNull(connections.poll(20, TimeUnit.SECONDS));
            assertNull(updates.poll(1, TimeUnit.SECONDS));
            Files.writeString(source, "{\"afterRestart\":true}");
            var third = updates.poll(10, TimeUnit.SECONDS);
            assertNotNull(third);
            verifyDownload(third);
        } finally {
            server.destroyForcibly();
            assertTrue(server.waitFor(5, TimeUnit.SECONDS));
        }
    }

    private Process launch() throws Exception {
        var builder = new ProcessBuilder(System.getenv("RPP_BIN"), "-C", project.toString(), "dev");
        builder.environment().put("RPP_HOME", project.resolve("user").toString());
        return builder.redirectErrorStream(true)
                .redirectOutput(
                        ProcessBuilder.Redirect.appendTo(project.resolve("server.log").toFile()))
                .start();
    }

    private static void verifyDownload(PackUpdate pack) throws Exception {
        try (var stream = pack.url().toURL().openStream()) {
            byte[] zip = stream.readAllBytes();
            assertEquals(pack.size(), zip.length);
            assertEquals(
                    pack.sha1(),
                    HexFormat.of().formatHex(MessageDigest.getInstance("SHA-1").digest(zip)));
        }
    }
}
