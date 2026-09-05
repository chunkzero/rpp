package dev.chunkzero.rpp;

import java.net.URI;
import java.util.HexFormat;
import java.util.Objects;

/**
 * Metadata for one immutable development pack, suitable for Minecraft pack offers.
 * @param url player-reachable download URL
 * @param sha1 lowercase hexadecimal SHA-1 of the exact ZIP bytes
 * @param size ZIP size in bytes
 */
public record PackUpdate(URI url, String sha1, long size) {
    /** Validates the download URL, SHA-1, and byte count. */
    public PackUpdate {
        Objects.requireNonNull(url, "url");
        Objects.requireNonNull(sha1, "sha1");
        if (!("http".equals(url.getScheme()) || "https".equals(url.getScheme()))
                || url.getHost() == null || !sha1.matches("[0-9a-f]{40}") || size < 0) {
            throw new IllegalArgumentException("Invalid pack URL, SHA-1, or size");
        }
    }

    /**
     * Returns the hash for APIs such as Spigot's setResourcePack.
     * @return a fresh 20-byte SHA-1 array
     */
    public byte[] hashBytes() {
        return HexFormat.of().parseHex(sha1);
    }
}
