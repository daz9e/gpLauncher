// `use:whenVisible={callback}`: calls back each time the element scrolls into view, e.g. to load
// the next page of a list.

export function whenVisible(node: HTMLElement, callback: () => void) {
  let current = callback;
  const observer = new IntersectionObserver(
    (entries) => entries.some((e) => e.isIntersecting) && current(),
    { rootMargin: '200px' },
  );
  observer.observe(node);
  return {
    update(next: () => void) {
      current = next;
    },
    destroy() {
      observer.disconnect();
    },
  };
}
