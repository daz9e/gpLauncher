// Icons shared with the rest of the project (Lucide, ISC license; brand logos from Simple Icons, CC0).

const files = import.meta.glob('../../../assets/icons/*.svg', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

export const icons: Record<string, string> = {};
for (const [path, raw] of Object.entries(files)) {
  const name = path.split('/').pop()!.replace(/\.svg$/, '');
  // Size comes from the wrapper; only the root element's width and height go.
  const svg = raw.replace(/<svg[^>]*>/, (tag) => tag.replace(/ (width|height)="\d+"/g, ''));
  icons[name] = svg
    .replace(/stroke="#000"/g, 'stroke="currentColor"')
    .replace('<svg ', raw.includes('stroke=') ? '<svg ' : '<svg fill="currentColor" ');
}
