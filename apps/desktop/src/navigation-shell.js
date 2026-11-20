import { mountNavigationSidebar } from '../../../design-system/navigation.mjs';
import { mountSidebarViews } from './sidebar-views.js';

// The app's one owner of navigation: the shared shell decides whether the sidebar shows,
// the sidebar views decide which list it shows, and this joins the two.
// `onPreview` fills the list the rail named; `onActivate` follows a rail click or key;
// `sync(route)` is called after each screen render with where the content now is.
export function mountNavigationShell({ document, storage, icon, count, folderName, onPreview, onActivate = () => {}, onNotifications }) {
  const byId = id => document.getElementById(id);
  const aside = byId('navigation-sidebar');
  let views = null, route = null;
  const sidebar = mountNavigationSidebar({
    shell: document.querySelector('.workspace'), aside, content: document.querySelector('.main-content'), trigger: byId('navigation-toggle'), storage, icons: icon,
    onPreview: view => views?.choose(view),
    onActivate,
    onStateChange: () => views?.refresh(),
  });
  views = mountSidebarViews({
    aside, navigation: byId('sidebar-navigation-view'), notifications: byId('notification-center'), trigger: byId('notifications-open'), badge: byId('notification-count'), storage,
    count, folderName, onPreview, onChange: onNotifications, reveal: () => sidebar.reveal(),
  });
  // The list remembered from the last session is filled from the start.
  onPreview(views.activeView(), views.folder());
  return {
    // A temporary sidebar closes once the content moves somewhere else.
    sync(next) {
      sidebar.render();
      if (route !== null && route !== next) sidebar.navigate();
      route = next;
      views.refresh();
    },
    // The person chose a place again, even the one already shown.
    navigated: () => sidebar.navigate(),
    attention: (view, on) => sidebar.attention(view, on),
    updateCounts: counts => views.updateCounts(counts),
    choose: (view, folder) => views.choose(view, folder),
    close: options => views.close(options),
    open: options => views.open(options),
    isOpen: () => views.isOpen(),
    activeView: () => views.activeView(),
    folder: () => views.folder(),
    destroy() { views.destroy(); sidebar.dispose(); },
  };
}
