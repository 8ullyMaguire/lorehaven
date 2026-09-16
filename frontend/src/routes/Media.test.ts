import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import Media from './Media.svelte';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
it('loads eligible media and keeps subscription links tied to the query', async () => {
  const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ items: [{id:'work-1', title:'A quiet archive', summary:'A journey home.', format:'prose', rating:'general'}], total:1, next_cursor:null }), {headers:{'Content-Type':'application/json'}}));
  vi.stubGlobal('fetch', fetch);
  render(Media);
  expect(await screen.findByRole('link', {name:'A quiet archive'})).toHaveAttribute('href', '/works/work-1');
  await fireEvent.input(screen.getByLabelText('Search media'), {target:{value:'title:archive'}});
  await fireEvent.submit(screen.getByRole('search'));
  expect(await screen.findByRole('link', {name:'Atom feed'})).toHaveAttribute('href', '/api/v1/media/feed?q=title%3Aarchive&format=atom');
});
it('provides a useful empty state', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({items:[], total:0, next_cursor:null}), {headers:{'Content-Type':'application/json'}})));
  render(Media);
  expect(await screen.findByText('No media found')).toBeInTheDocument();
});
