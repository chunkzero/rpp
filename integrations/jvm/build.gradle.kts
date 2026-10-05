plugins {
    `java-library`
    `maven-publish`
}

group = "com.chunkzero.rpp"
// Release builds pass the full version; otherwise it's the Cargo workspace's upcoming release.
version =
    providers
        .environmentVariable("RPP_RELEASE_VERSION")
        .orElse(
            providers.fileContents(layout.projectDirectory.file("../../Cargo.toml")).asText.map {
                Regex("""(?m)^version = "(.+)"$""").find(it)!!.groupValues[1]
            },
        ).get()

java {
    toolchain { languageVersion = JavaLanguageVersion.of(21) }
    withSourcesJar()
    withJavadocJar()
}

dependencies {
    implementation(libs.jackson)
    testImplementation(libs.junit)
    testRuntimeOnly(libs.junit.launcher)
}

tasks.test { useJUnitPlatform() }
tasks.withType<JavaCompile>().configureEach {
    options.compilerArgs.addAll(listOf("-Xlint:all", "-Werror"))
}
publishing {
    publications {
        create<MavenPublication>("client") { from(components["java"]) }
    }
    // `maven-r2 publish` supplies the local publication proxy.
    providers.environmentVariable("MAVEN_R2_URL").orNull?.let { proxy ->
        repositories {
            maven {
                name = "MavenR2"
                url = uri(proxy)
                isAllowInsecureProtocol = true // The proxy listens on loopback only.
                credentials {
                    username = providers.environmentVariable("MAVEN_R2_USERNAME").get()
                    password = providers.environmentVariable("MAVEN_R2_PASSWORD").get()
                }
            }
        }
    }
}
dependencyLocking { lockAllConfigurations() }
