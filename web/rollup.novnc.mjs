import resolve from "@rollup/plugin-node-resolve";
import commonjs from "@rollup/plugin-commonjs";

export default {
  input: "node_modules/@novnc/novnc/lib/rfb.js",
  output: {
    file: "public/novnc-rfb.js",
    format: "iife",
    name: "__noVNC",
    exports: "named",
  },
  plugins: [
    resolve({ browser: true }),
    commonjs(),
  ],
};
