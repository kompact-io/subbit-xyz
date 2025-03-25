export function inOrder(q: (() => Promise<any>)[]) {
  const j = q.shift();
  if (j == undefined) return;
  return Promise.resolve(j()).then(() => inOrder(q));
}
