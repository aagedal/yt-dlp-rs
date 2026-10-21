#!/usr/bin/env python3
"""Capture upstream ModifyChaptersPP chapter arithmetic for native fixtures.

Development/reference tooling only; Python is never a runtime fallback. The
pinned upstream checkout's own TestModifyChaptersPP cases are run with their
assertions recorded instead: each `_remove_marked_arrange_sponsors` input with
upstream's resulting chapters (start, end, title) and cuts (start, end), each
`_make_concat_opts` input and result, and `_quote_for_ffmpeg` strings. No
network access is involved.

usage: capture_modify_chapters.py UPSTREAM_CHECKOUT OUTPUT_JSON
"""
import copy
import json
import sys
import unittest


def main():
    checkout, output = sys.argv[1:]
    sys.path.insert(0, checkout)
    sys.path.insert(0, f'{checkout}/test')
    import test_postprocessors as tests
    from yt_dlp.postprocessor.modify_chapters import ModifyChaptersPP

    arrange, concat, quotes = [], [], []
    case = tests.TestModifyChaptersPP

    def arrange_impl(self, chapters, expected_chapters, expected_removed):
        given = copy.deepcopy(chapters)
        actual, removed = self._pp._remove_marked_arrange_sponsors(chapters)
        arrange.append({
            'title': self._pp._sponsorblock_chapter_title,
            'chapters': given,
            'expected': [[c['start_time'], c['end_time'], c['title']] for c in actual],
            'cuts': [[c['start_time'], c['end_time']] for c in removed],
        })

    case._remove_marked_arrange_sponsors_test_impl = arrange_impl
    original_concat = ModifyChaptersPP._make_concat_opts

    def make_concat_opts(chapters, duration):
        result = original_concat(chapters, duration)
        concat.append({
            'cuts': [[c['start_time'], c['end_time']] for c in chapters],
            'duration': duration,
            'expected': result,
        })
        return result

    ModifyChaptersPP._make_concat_opts = staticmethod(make_concat_opts)
    original_quote = ModifyChaptersPP._quote_for_ffmpeg

    def quote(string):
        result = original_quote(string)
        quotes.append([string, result])
        return result

    ModifyChaptersPP._quote_for_ffmpeg = staticmethod(quote)
    suite = unittest.TestLoader().loadTestsFromTestCase(case)
    result = unittest.TestResult()
    suite.run(result)
    for _, trace in result.errors:
        print(trace, file=sys.stderr)
    # The tuple-typed categories serialize as lists.
    with open(output, 'w', encoding='utf-8') as file:
        json.dump({'arrange': arrange, 'concat': concat, 'quote': quotes}, file, indent=1)
        file.write('\n')
    print(f'{len(arrange)} arrange, {len(concat)} concat, {len(quotes)} quote cases')


if __name__ == '__main__':
    main()
