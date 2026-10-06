# httpstatr integration pilot

The local pilot uses D:\sriram\httpstatr\target\debug\httpstatr.exe, which reports
httpstatr 1.0.0. The neighboring release binary reports an older 1.0.0-rc.1 and
is excluded. Binary hashes, curl version/hash, Python/server version and generated
suite hashes are recorded in each evidence manifest.

```powershell
python tools/accept-httpstatr.py --runner target/release/spanforge-verify.exe --target D:/sriram/httpstatr/target/debug/httpstatr.exe --curl C:/Windows/System32/curl.exe
```

The external server binds an assigned loopback port and provides fixed success,
HTTP 418 mismatch and delayed routes. The generated suite checks help, exact
version, invalid format arguments, HTTP assertions, httpstatr's own deadline,
JSON/report output and absence of a synthetic credential. The harness separately
validates a variable target report file, then changes a status expectation and
proves that the regression produces runner exit 1. No case setup hooks or public
endpoints are used. The target receives an absolute curl path; proxy variables
are excluded by the runner's environment policy.

The target deadline case expects httpstatr exit 28 and its request-deadline message,
with no runner termination reason. A runner timeout would fail the contract.
Invalid known --format values exit 2; arbitrary unknown flags may be forwarded by
httpstatr and are unsuitable for that test objective.

Eight cases and the intentional regression passed locally on 6 October 2026.
Evidence is under target/httpstatr-evidence. Generated suites are concrete and
reviewable; expectations are never automatically learned from target output.
This is a local 1.0.0 binary pilot, not certification of a signed published release
or Linux httpstatr support. Review the pinned contracts before general release.
