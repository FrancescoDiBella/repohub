# R8 rules for the release build, on top of proguard-android-optimize.txt.

# The page calls these by name through `RepoHubNative` (MainActivity.Bridge). The default rules
# keep @JavascriptInterface methods already; this says so where it matters.
-keepclassmembers class io.github.francescodibella.repohub.MainActivity$Bridge {
    @android.webkit.JavascriptInterface <methods>;
}
