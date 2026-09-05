plugins { application }

java { toolchain { languageVersion = JavaLanguageVersion.of(25) } }
dependencies {
    implementation(project(":"))
    implementation(libs.minestom)
}
application { mainClass = "dev.chunkzero.rpp.example.PackServer" }
dependencyLocking { lockAllConfigurations() }
