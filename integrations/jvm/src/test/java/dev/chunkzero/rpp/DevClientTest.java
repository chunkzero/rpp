package dev.chunkzero.rpp;

import java.io.IOException;
import java.net.InetSocketAddress;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

import com.sun.net.httpserver.HttpServer;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

class DevClientTest {
    private static final String A = "a".repeat(40);
    private static final String B = "b".repeat(40);

    private static String reload(String hash) {
        return "data: {\"type\":\"reload\",\n"
                + "data: \"pack\":{\"sha1\":\"" + hash + "\",\"url\":\"/packs/" + hash
                + ".zip\",\"size\":123}}\n\n";
    }

    @Test
    void reconnects_deduplicates_and_reports_build_and_update_failures() throws Exception {
        var requests = new AtomicInteger();
        var attempts = new AtomicInteger();
        var updates = new LinkedBlockingQueue<PackUpdate>();
        var failures = new LinkedBlockingQueue<DevClient.FailureKind>();
        var finished = new CountDownLatch(1);
        var hold = new CountDownLatch(1);
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/events", exchange -> {
            int attempt = requests.incrementAndGet();
            if (attempt == 1) {
                exchange.sendResponseHeaders(503, -1);
                exchange.close();
                return;
            }
            exchange.getResponseHeaders().set("Content-Type", "text/event-stream; charset=utf-8");
            exchange.sendResponseHeaders(200, 0);
            try (var output = exchange.getResponseBody()) {
                String events = attempt == 2
                        ? ": keepalive\n\n" + reload(A) + reload(A)
                            + "data: {\"type\":\"build_error\",\"message\":\"bad Lua\"}\n\n"
                            + "data: {\"type\":\"reload\",\"changed\":[\"x\"]}\n\n" + reload(B)
                        : reload(B);
                output.write(events.getBytes(StandardCharsets.UTF_8));
                output.flush();
                if (attempt >= 3) {
                    finished.countDown();
                    try {
                        hold.await(5, TimeUnit.SECONDS);
                    } catch (InterruptedException interrupted) {
                        Thread.currentThread().interrupt();
                    }
                }
            } finally {
                exchange.close();
            }
        });
        server.start();
        URI base = URI.create("http://127.0.0.1:" + server.getAddress().getPort());
        try (var client = new DevClient(base, URI.create("https://packs.example/prefix/"),
                Duration.ofMillis(20), new DevClient.Listener() {
                    @Override
                    public void onUpdate(PackUpdate update) throws Exception {
                        if (update.sha1().equals(B) && attempts.incrementAndGet() == 1) {
                            throw new IOException("Offer failed");
                        }
                        updates.add(update);
                    }

                    @Override
                    public void onFailure(DevClient.FailureKind kind, Exception failure) {
                        failures.add(kind);
                    }
                })) {
            client.start();
            assertEquals(A, java.util.Objects.requireNonNull(updates.poll(5, TimeUnit.SECONDS)).sha1());
            var second = updates.poll(5, TimeUnit.SECONDS);
            assertNotNull(second);
            assertEquals(B, second.sha1());
            assertEquals(URI.create("https://packs.example/prefix/packs/" + B + ".zip"), second.url());
            assertEquals(20, second.hashBytes().length);
            assertTrue(finished.await(5, TimeUnit.SECONDS));
            assertEquals(DevClient.FailureKind.CONNECTION, failures.poll(5, TimeUnit.SECONDS));
            assertEquals(DevClient.FailureKind.BUILD, failures.poll(5, TimeUnit.SECONDS));
            assertEquals(DevClient.FailureKind.UPDATE, failures.poll(5, TimeUnit.SECONDS));
            assertTrue(updates.isEmpty());
            assertTrue(failures.isEmpty());
            assertTimeout(Duration.ofSeconds(1), client::close);
            hold.countDown();
            assertNull(updates.poll(100, TimeUnit.MILLISECONDS));
        } finally {
            hold.countDown();
            server.stop(0);
        }
    }

    @Test
    void malformed_metadata_is_observable_and_retried() throws Exception {
        var failures = new LinkedBlockingQueue<DevClient.FailureKind>();
        var updates = new LinkedBlockingQueue<PackUpdate>();
        var requests = new AtomicInteger();
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/events", exchange -> {
            exchange.getResponseHeaders().set("Content-Type", "text/event-stream");
            exchange.sendResponseHeaders(200, 0);
            try (var out = exchange.getResponseBody()) {
                String data = requests.incrementAndGet() == 1
                        ? reload(A).replace("/packs/", "https://unexpected.example/packs/") : reload(A);
                out.write(data.getBytes(StandardCharsets.UTF_8));
            }
        });
        server.start();
        URI base = URI.create("http://127.0.0.1:" + server.getAddress().getPort());
        try (var client = new DevClient(base, base, Duration.ofMillis(20), new DevClient.Listener() {
            @Override
            public void onUpdate(PackUpdate pack) { updates.add(pack); }

            @Override
            public void onFailure(DevClient.FailureKind kind, Exception failure) { failures.add(kind); }
        })) {
            client.start();
            assertEquals(DevClient.FailureKind.PROTOCOL, failures.poll(5, TimeUnit.SECONDS));
            assertNotNull(updates.poll(5, TimeUnit.SECONDS));
        } finally {
            server.stop(0);
        }
    }
}
