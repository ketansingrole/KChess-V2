module.exports = async ({ appOutDir, electronPlatformName }) => {
  const { verifyPackage } = await import('./verify-package.mjs')
  await verifyPackage(appOutDir, electronPlatformName)
}
