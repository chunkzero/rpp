plugins { application }

java { toolchain { languageVersion = JavaLanguageVersion.of(25) } }
dependencies {
    implementation(project(":"))
    implementation(libs.minestom)
}
application { mainClass = "com.chunkzero.rpp.example.PackServer" }
dependencyLocking { lockAllConfigurations() }
