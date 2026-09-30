import deckyPlugin from "@decky/rollup";
import typescript from "@rollup/plugin-typescript";
const config = deckyPlugin({});
config.plugins = config.plugins.map(plugin => plugin.name === "typescript"
  ? typescript({ noEmit: false, filterRoot: "../..", include: ["apps/decky/src/**/*", "packages/ui/src/**/*"] })
  : plugin);
export default config;
