const brandIcons = {
  chatgpt: '/shared/assets/brands/chatgpt.png',
  claude: '/shared/assets/brands/claude.png',
  notion: '/shared/assets/brands/notion.png',
  calendar: '/shared/assets/brands/google-calendar.webp',
};

export function brandIconUrl(name) {
  return brandIcons[name];
}

export function mountBrandIcons(root) {
  root.querySelectorAll('img[data-brand-icon]').forEach(image => {
    image.src = brandIconUrl(image.dataset.brandIcon);
  });
}
