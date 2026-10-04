three.min.js: three.js r186 (npm three@0.186.1), MIT (three-LICENSE.txt).
Subset bundled and minified with esbuild from site/vendor/three-entry.js; only tachyon-3d.js imports it.
Rebuild: npm i three@0.186.1 esbuild && npx esbuild site/vendor/three-entry.js --bundle --minify --format=esm --legal-comments=eof --outfile=site/vendor/three.min.js
