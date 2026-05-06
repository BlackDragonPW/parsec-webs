pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
        // Mozilla rust-android-gradle plugin is published to gradle plugin portal
        // but also needs mavenCentral for transitive deps — both already listed above.
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "parsec-browser"
include(":app")
