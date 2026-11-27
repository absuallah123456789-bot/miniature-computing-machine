#!/usr/bin/env python3
"""Post-`tauri android init` patcher.

0. Copies ChesLan launcher icons + makes MainActivity respect system bars (see below).
1. AndroidManifest.xml: add the network permissions ChesLan needs for LAN play.
2. app/build.gradle.kts: add a release signingConfig driven by env vars (see below).

Idempotent. Run from the repo root after `npx tauri android init`.
"""
import re, sys, pathlib

root = pathlib.Path("src-tauri/gen/android")
manifest = root / "app/src/main/AndroidManifest.xml"
gradle = root / "app/build.gradle.kts"
ok = True

# ---- manifest ----
PERMS = [
    "android.permission.INTERNET",                     # TCP game channel + UDP discovery sockets
    "android.permission.ACCESS_NETWORK_STATE",
    "android.permission.ACCESS_WIFI_STATE",
    "android.permission.CHANGE_WIFI_MULTICAST_STATE",  # harmless; lets future mDNS code hold a MulticastLock
]
if manifest.exists():
    text = manifest.read_text()
    missing = [p for p in PERMS if p not in text]
    if missing:
        add = "".join(f'    <uses-permission android:name="{p}" />\n' for p in missing)
        text, n = re.subn(r"(\s*<application)", "\n" + add.rstrip("\n") + r"\n\1", text, count=1)
        if n != 1:
            print("!! could not find <application> in manifest", file=sys.stderr); ok = False
        manifest.write_text(text)
    print("manifest: permissions ok")
else:
    print(f"!! {manifest} not found (run `npx tauri android init` first)", file=sys.stderr); ok = False

# ---- gradle signing (same env-var scheme as MalikHw/vidasscreensaver) ----
# KEYSTORE_FILE / KEYSTORE_PASSWORD / KEY_ALIAS / KEY_PASSWORD; falls back to debug signing
# unless REQUIRE_RELEASE_SIGNING=true (then the build fails instead of silently using debug keys).
MARK = "// cheslan-signing"
if gradle.exists():
    g = gradle.read_text()
    if MARK not in g:
        signing = f'''
    {MARK}
    signingConfigs {{
        create("release") {{
            val ksFile = System.getenv("KEYSTORE_FILE")
            val requireSign = System.getenv("REQUIRE_RELEASE_SIGNING") == "true"
            if (ksFile != null && file(ksFile).exists()) {{
                storeFile = file(ksFile)
                storePassword = System.getenv("KEYSTORE_PASSWORD")
                keyAlias = System.getenv("KEY_ALIAS")
                keyPassword = System.getenv("KEY_PASSWORD")
            }} else if (requireSign) {{
                throw GradleException("Release signing required but KEYSTORE_FILE is unset or missing (KEYSTORE_FILE=$ksFile).")
            }} else {{
                initWith(signingConfigs.getByName("debug"))
            }}
        }}
    }}
'''
        g, n1 = re.subn(r"(?m)^(\s*)buildTypes\s*\{", signing + r"\1buildTypes {", g, count=1)
        g, n2 = re.subn(r'(getByName\("release"\)\s*\{)', r'\1\n            signingConfig = signingConfigs.getByName("release")', g, count=1)
        if not (n1 and n2):
            print(f"!! gradle patch incomplete ({n1},{n2}); check generated build.gradle.kts", file=sys.stderr); ok = False
        gradle.write_text(g)
    print("gradle: signing config ok")
else:
    print(f"!! {gradle} not found", file=sys.stderr); ok = False

# ---- app icon: copy the committed ChesLan launcher icons over Tauri's defaults ----
import shutil
res = root / "app/src/main/res"
icons = pathlib.Path("src-tauri/icons/android")
if res.exists() and icons.exists():
    for f in icons.rglob("*"):
        if f.is_file():
            dst = res / f.relative_to(icons)
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(f, dst)
    print("icons: ChesLan launcher icons installed")
else:
    print("!! res dir or src-tauri/icons/android missing", file=sys.stderr); ok = False

# ---- system bars: pad the content view by status bar / navigation bar / cutout insets ----
IMARK = "// cheslan-insets"
kt = next(iter(root.glob("app/src/main/java/**/MainActivity.kt")), None)
if kt:
    k = kt.read_text()
    if IMARK not in k:
        block = f"""
        {IMARK}
        val content = findViewById<android.view.View>(android.R.id.content)
        androidx.core.view.ViewCompat.setOnApplyWindowInsetsListener(content) {{ v, insets ->
            val b = insets.getInsets(
                androidx.core.view.WindowInsetsCompat.Type.systemBars() or
                androidx.core.view.WindowInsetsCompat.Type.displayCutout())
            v.setPadding(b.left, b.top, b.right, b.bottom)
            androidx.core.view.WindowInsetsCompat.CONSUMED
        }}
"""
        if "super.onCreate(savedInstanceState)" in k:
            k = k.replace("super.onCreate(savedInstanceState)", "super.onCreate(savedInstanceState)" + block, 1)
        else:
            override = ("override fun onCreate(savedInstanceState: android.os.Bundle?) {\n"
                        "        super.onCreate(savedInstanceState)" + block + "    }\n")
            m = re.search(r"class MainActivity[^\n{]*\{", k)
            if m:  # class already has a body
                k = k[:m.end()] + "\n    " + override + k[m.end():]
            else:  # bare `class MainActivity : TauriActivity()`
                k = re.sub(r"(class MainActivity[^\n{]*)", lambda mm: mm.group(1) + " {\n    " + override + "}", k, count=1)
        kt.write_text(k)
    print("insets: MainActivity patched")
else:
    print("!! MainActivity.kt not found", file=sys.stderr); ok = False

sys.exit(0 if ok else 1)
