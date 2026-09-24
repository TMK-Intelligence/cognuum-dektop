export const REPOSITORY = 'TMK-Intelligence/cognuum-dektop';
export function validateConfig(config, channel = 'production') {
  const expected = channel === 'production'
    ? ['https://access.cognuum.com', 'cognuum', 'com.cognuum.desktop']
    : ['https://staging-access.cognuum.com', 'cognuum-staging', 'com.cognuum.desktop.staging'];
  if (!['production', 'staging'].includes(channel)) throw new Error('Unknown channel');
  const desktop = config.plugins?.desktop;
  if (desktop?.origin !== expected[0] || desktop?.channel !== channel || desktop?.scheme !== expected[1] || config.identifier !== expected[2]) throw new Error('Desktop environment mismatch');
  if (config.app.withGlobalTauri !== false || config.app.security.capabilities.length !== 0) throw new Error('Remote native IPC must remain disabled');
  if (config.app.windows.length !== 0) throw new Error('Windows must use the guarded native builder');
  if (config.plugins['deep-link'].desktop.schemes.join(',') !== expected[1]) throw new Error('Deep-link channel mismatch');
}

export function releaseConfig(channel, platform, env) {
  if (!['production', 'staging'].includes(channel)) throw new Error('Unknown channel');
  const names = ['TAURI_UPDATER_PUBLIC_KEY', 'TAURI_SIGNING_PRIVATE_KEY'];
  if (platform === 'darwin') names.push('APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID');
  else if (platform === 'win32') names.push('WINDOWS_SIGN_COMMAND');
  else throw new Error('Only macOS and Windows are supported');
  const missing = names.filter(name => !env[name]?.trim());
  if (missing.length) throw new Error(`Missing release configuration: ${missing.join(', ')}`);
  if (!/^[A-Za-z0-9+/=]+$/.test(env.TAURI_UPDATER_PUBLIC_KEY)) throw new Error('Invalid updater public key');
  const endpoint = channel === 'production'
    ? `https://github.com/${REPOSITORY}/releases/latest/download/latest.json`
    : `https://github.com/${REPOSITORY}/releases/download/staging/latest.json`;
  return {
    bundle: { createUpdaterArtifacts: true, ...(platform === 'win32' ? { windows: { signCommand: env.WINDOWS_SIGN_COMMAND } } : {}) },
    plugins: { updater: { pubkey: env.TAURI_UPDATER_PUBLIC_KEY, endpoints: [endpoint] } },
  };
}
