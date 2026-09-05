package dev.chunkzero.rpp;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.net.HttpURLConnection;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.Objects;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

/**
 * Reconnecting RPP SSE client. Callbacks run serially on a private daemon thread.
 * Close on integration shutdown. Platform scheduling and player responses belong to callers.
 */
public final class DevClient implements AutoCloseable {
    /** Distinguishes transport, malformed metadata, build, and caller update failures. */
    public enum FailureKind {
        /** HTTP, network, idle timeout, or closed event stream. */
        CONNECTION,
        /** Malformed JSON or invalid pack metadata. */
        PROTOCOL,
        /** RPP reported a failed rebuild; the previous pack remains available. */
        BUILD,
        /** The caller's update callback threw. */
        UPDATE
    }

    /** Callbacks must return promptly. Failure callbacks must not throw. */
    public interface Listener {
        /** Called after each successful SSE connection, including reconnects. */
        default void onConnected() {}

        /**
         * Offer this pack or schedule an offer on the server thread.
         * Throwing reports UPDATE and reconnects to retry the latest snapshot.
         * @param pack the latest available pack
         * @throws Exception if accepting or scheduling the update fails
         */
        void onUpdate(PackUpdate pack) throws Exception;

        /**
         * Reports failures; connection/protocol/update failures retry after the configured delay.
         * @param kind the failing operation
         * @param failure the cause or server diagnostic
         */
        void onFailure(FailureKind kind, Exception failure);
    }

    private final URI events;
    private final URI downloadBase;
    private final Listener listener;
    private final long retryMillis;
    private final ObjectMapper json = new ObjectMapper();
    private final Thread worker;
    private volatile boolean closed;
    private volatile PackUpdate latest;

    /**
     * Creates an unstarted client. Both bases are HTTP(S) origins (optionally with a path prefix).
     * downloadBase must be reachable by players; it may differ from the server-side event origin.
     * @param serverBase base URL of the RPP event server
     * @param downloadBase player-reachable base URL for pack downloads
     * @param reconnectDelay fixed retry delay, at least one millisecond
     * @param listener ordered connection, update, and failure callbacks
     */
    public DevClient(URI serverBase, URI downloadBase, Duration reconnectDelay, Listener listener) {
        this.events = base(serverBase).resolve("events");
        this.downloadBase = base(downloadBase);
        this.listener = Objects.requireNonNull(listener, "listener");
        this.retryMillis = Objects.requireNonNull(reconnectDelay, "reconnectDelay").toMillis();
        if (retryMillis < 1) throw new IllegalArgumentException("Reconnect delay must be positive");
        this.worker = Thread.ofVirtual().name("rpp-dev-client").unstarted(this::run);
    }

    private static URI base(URI uri) {
        Objects.requireNonNull(uri, "base");
        if (!("http".equals(uri.getScheme()) || "https".equals(uri.getScheme()))
                || uri.getHost() == null || uri.getQuery() != null || uri.getFragment() != null
                || uri.getUserInfo() != null) {
            throw new IllegalArgumentException("Expected an HTTP(S) base URL");
        }
        return URI.create(uri.toString().replaceAll("/+$", "") + "/");
    }

    /** Starts once; create a new client after closing. */
    public synchronized void start() {
        if (closed) throw new IllegalStateException("Client is closed");
        worker.start();
    }

    /**
     * Returns the last update accepted by the callback.
     * @return the accepted update, or null before the first successful callback
     */
    public PackUpdate latest() {
        return latest;
    }

    private void run() {
        while (!closed) {
            try {
                if (listen() && !closed) report(FailureKind.CONNECTION, new IOException("Event stream ended"));
            } catch (Exception failure) {
                if (!closed) report(FailureKind.CONNECTION, failure);
            }
            if (!closed) {
                try {
                    Thread.sleep(retryMillis);
                } catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    return;
                }
            }
        }
    }

    private boolean listen() throws IOException {
        HttpURLConnection active = (HttpURLConnection) events.toURL().openConnection();
        if (closed) return false;
        active.setConnectTimeout(10_000);
        active.setReadTimeout(45_000); // RPP emits SSE keepalives every 15 seconds.
        active.setInstanceFollowRedirects(false);
        active.setRequestProperty("Accept", "text/event-stream");
        try {
            if (active.getResponseCode() != 200) {
                throw new IOException("SSE HTTP status " + active.getResponseCode());
            }
            String contentType = active.getContentType();
            if (contentType == null || !contentType.split(";")[0].trim().equals("text/event-stream")) {
                throw new IOException("Expected text/event-stream");
            }
            if (closed) return false;
            listener.onConnected();
            try (var reader = new BufferedReader(new InputStreamReader(
                    active.getInputStream(), StandardCharsets.UTF_8))) {
                StringBuilder data = new StringBuilder();
                String line;
                while (!closed && (line = reader.readLine()) != null) {
                    if (line.isEmpty()) {
                        if (!data.isEmpty() && !dispatch(data.toString())) return false;
                        data.setLength(0);
                    } else if (line.equals("data") || line.startsWith("data:")) {
                        String value = line.length() > 5 ? line.substring(5) : "";
                        if (value.startsWith(" ")) value = value.substring(1);
                        if (!data.isEmpty()) data.append('\n');
                        data.append(value);
                        if (data.length() > 1_048_576) throw new IOException("SSE event exceeds 1 MiB");
                    }
                }
            }
        } finally {
            active.disconnect();
        }
        return true;
    }

    private boolean dispatch(String data) {
        PackUpdate update;
        try {
            JsonNode event = json.readTree(data);
            if (event == null) throw new IOException("Empty event");
            String type = event.path("type").asText();
            if (type.equals("build_error")) {
                report(FailureKind.BUILD, new IOException(event.path("message").asText("Build failed")));
                return true;
            }
            if (!type.equals("reload") || event.path("pack").isNull()) return true;
            JsonNode pack = event.get("pack");
            if (pack == null) return true; // Changed-path-only events remain supported.
            String hash = pack.path("sha1").asText();
            String path = pack.path("url").asText();
            if (!hash.matches("[0-9a-f]{40}") || !path.equals("/packs/" + hash + ".zip")
                    || !pack.path("size").isIntegralNumber() || !pack.path("size").canConvertToLong()) {
                throw new IOException("Invalid pack metadata");
            }
            update = new PackUpdate(downloadBase.resolve(path.substring(1)), hash, pack.path("size").longValue());
        } catch (Exception failure) {
            report(FailureKind.PROTOCOL, failure);
            return false;
        }
        if (closed || (latest != null && latest.sha1().equals(update.sha1()))) return true;
        try {
            listener.onUpdate(update);
            latest = update;
            return true;
        } catch (Exception failure) {
            report(FailureKind.UPDATE, failure);
            return false;
        }
    }

    private void report(FailureKind kind, Exception failure) {
        if (closed) return;
        try {
            listener.onFailure(kind, failure);
        } catch (RuntimeException callbackFailure) {
            // A broken error handler must not silently kill the reconnect worker.
            System.getLogger(DevClient.class.getName()).log(
                    System.Logger.Level.ERROR, "RPP failure callback threw", callbackFailure);
        }
    }

    /** Stops reconnects and disconnects the stream. An already-running callback may finish. */
    @Override
    public synchronized void close() {
        closed = true;
        worker.interrupt();
    }
}
