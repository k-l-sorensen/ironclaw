import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const dir = path.dirname(fileURLToPath(import.meta.url));

// The NEAR wallet sign-in popup is detected as cancelled by polling
// `popup.closed` (awaitWalletSignature). `window.open` returns null when opened
// with 'noopener'/'noreferrer', which would make that poll impossible and leave
// a cancelled sign-in dead-waiting the full timeout. The popup is a same-origin
// app page that talks back over BroadcastChannel, so it must open WITHOUT those
// flags to keep a real handle.
test('wallet login popup opens without noopener/noreferrer so close-detection works', async () => {
  const source = await readFile(path.join(dir, 'useProviderLogin.js'), 'utf8');

  // The wallet popup open uses a plain size feature string, no opener-nulling
  // flags. Quote style may vary (mono double / desktop single).
  assert.match(
    source,
    /window\.open\(\s*walletLoginUrl\(channelName\),\s*['"]_blank['"],\s*['"]width=460,height=640['"]\s*\)/
  );
  assert.doesNotMatch(source, /['"]noopener,noreferrer,width/);

  // The cancel path still relies on the popup handle being real.
  assert.match(source, /if \(popup && popup\.closed\)/);
});
