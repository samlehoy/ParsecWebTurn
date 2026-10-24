"""Build-only comparison patch; never part of the application executable."""
from pathlib import Path
import subprocess

BASE = "323c5757f47f4073aae1ecaf15073b5a3ddd69e8"
REFERENCE = "4f0b678e8dae4bf2f6e9772b082ee87edfeeee9e"
FILES = ["attempt.rs", "bud_attempt.rs", "stats.rs", "video_windows.rs"]
paths = ["src-native/src/" + name for name in FILES]
# Stop rather than reverting any later unrelated application changes.
subprocess.run(["git", "diff", "--exit-code", REFERENCE, "HEAD", "--", *paths], check=True)

def original(name):
    return subprocess.check_output(["git", "show", BASE + ":src-native/src/" + name]).decode("utf-8")

# In these three files the only changes since BASE are the new log fields,
# log-specific local and decoder-state log change predicates.
for name in FILES[:3]:
    Path("src-native/src/" + name).write_text(original(name), encoding="utf-8")

p = Path("src-native/src/video_windows.rs")
s = p.read_text(encoding="utf-8")
old = original("video_windows.rs")
start = s.index("pub fn hevc_444_available(")
end = s.index("// Validate decoder surfaces", start)
old_start = old.index("pub fn hevc_444_available(")
old_end = old.index("// A queue wake", old_start)
s = s[:start] + old[old_start:old_end] + s[end:]
start = s.index("            let fallback = shared", s.index("        if let Some(error) = failure"))
end = s.index("            crate::event_log::record(", start)
s = s[:start] + s[end:]
line = "                    \"decoder_fallback\":fallback,\n"
assert s.count(line) == 1
s = s.replace(line, "", 1)
# Both actual 4:4:4 fixes must survive the logging-only patch.
assert "validate_processor_texture(&td, subresource, (width, height))?;" in s
assert "DXGI_FORMAT_NV12 | DXGI_FORMAT_P010 | DXGI_FORMAT_AYUV" in s
p.write_text(s, encoding="utf-8")
diff = subprocess.check_output(["git", "diff", "--", *paths])
Path("dist/logging-comparison/B/source.patch").write_bytes(diff)
