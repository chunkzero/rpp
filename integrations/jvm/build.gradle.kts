plugins {
    `java-library`
    `maven-publish`
}

group = "com.chunkzero.rpp"
version = "0.1.0-alpha.0"

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
}
dependencyLocking { lockAllConfigurations() }
