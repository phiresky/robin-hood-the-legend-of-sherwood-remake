import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

const [inputHtmlPath = 'dist/index.html', outputHtmlPath = 'dist-inline/index.html'] =
    process.argv.slice(2);
const html = readFileSync(inputHtmlPath, 'utf8');
const scriptRe = /<script\s+type="module"\s+crossorigin\s+src="([^"]+)"><\/script>/;
const match = scriptRe.exec(html);

if (match === null) {
    console.error(`could not find Vite module script in ${inputHtmlPath}`);
    process.exit(1);
}

const src = match[1];
if (src === undefined) {
    console.error(`Vite module script in ${inputHtmlPath} had no src`);
    process.exit(1);
}

const scriptPath = join(dirname(inputHtmlPath), src.replace(/^\//, ''));
const js = readFileSync(scriptPath, 'utf8')
    .replace(/\n\/\/# sourceMappingURL=.*\.js\.map\s*$/u, '');
const inlined = html.replace(match[0], `<script type="module">\n${js}\n</script>`);

mkdirSync(dirname(outputHtmlPath), { recursive: true });
writeFileSync(outputHtmlPath, inlined);
console.log(`wrote ${outputHtmlPath}`);
