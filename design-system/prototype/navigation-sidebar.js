import { mountNavigationSidebar as mountSidebar } from '../navigation.mjs';

export const mountNavigationSidebar = options => mountSidebar({ storageKey: 'nero-proposal-sidebar-pinned', ...options });
