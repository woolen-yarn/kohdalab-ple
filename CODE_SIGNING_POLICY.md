# Code signing / 署名

Windows artifacts are currently unsigned. macOS bundles use local ad hoc signing, not Apple Developer ID/notarization. Publish SHA-256 files next to release assets. Do not copy TRKR's signing identities or claim its signing coverage for PLE.

Windows版は未署名、macOS版はローカルのアドホック署名です。Apple Developer ID署名・公証は行っていません。配布物とSHA-256を併置し、TRKRの署名名義や署名済み状態をPLEへ流用しません。
