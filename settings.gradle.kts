plugins {
    id("org.gradle.toolchains.foojay-resolver-convention") version "0.8.0"
}

rootProject.name = "rpp"
include("api:core")
include("api:minestom")
include("api:spigot")
include("examples:minestom")
