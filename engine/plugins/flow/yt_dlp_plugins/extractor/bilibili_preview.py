"""Add Bilibili part titles to Flow's flat previews using the existing page-list response."""
from urllib.parse import parse_qs, urlsplit

from yt_dlp.extractor.bilibili import BiliBiliIE

__all__ = []


class FlowBilibiliPreviewIE(BiliBiliIE, plugin_name='flow_preview'):
    def _download_json(self, url_or_request, *args, **kwargs):
        result = super()._download_json(url_or_request, *args, **kwargs)
        if isinstance(url_or_request, str) and '/x/player/pagelist' in url_or_request and isinstance(result, dict):
            self._flow_pages = result.get('data')
        return result

    def _real_extract(self, url):
        self._flow_pages = None
        result = super()._real_extract(url)
        pages = self._flow_pages
        if (not self.get_param('noplaylist') and 'p' not in parse_qs(urlsplit(url).query)
                and result.get('_type') == 'playlist' and isinstance(pages, list) and len(pages) > 1):
            bvid = result['id']
            entries = []
            for offset, part in enumerate(pages, 1):
                page = part.get('page') or offset
                entries.append({
                    '_type': 'url', 'ie_key': 'BiliBili',
                    'url': f'https://www.bilibili.com/video/{bvid}?p={page}',
                    'id': f'{bvid}_p{page}', 'playlist_index': page,
                    'title': part.get('part') or f'第 {page} P', 'duration': part.get('duration'),
                })
            result.update(entries=entries, playlist_count=len(entries))
        return result
