// plugins.gradle.org times out on slow connections; Maven Central has the
// same Kotlin plugin jars, so try it first.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
