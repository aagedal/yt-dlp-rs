import json
from pathlib import Path
import tempfile
import unittest

import extractor_tiers


LISTING = '''Alpha
  https://alpha.example/1
Beta (CURRENTLY BROKEN)
Gamma
html5
generic
  https://other.example/
'''


class ExtractorTiersTest(unittest.TestCase):
    def test_parse_listing(self):
        names, owners = extractor_tiers.parse_listing(LISTING)
        self.assertEqual(names, [('Alpha', False), ('Beta', True), ('Gamma', False),
                                 ('html5', False), ('generic', False)])
        self.assertEqual(owners, {'https://alpha.example/1': 'Alpha',
                                  'https://other.example/': 'generic'})

    def test_reached_names(self):
        text = json.dumps({'cases': [{'stdout': [
            '[Gamma] 1: Downloading webpage',
            json.dumps({'id': '1', 'extractor': 'Delta:x', 'extractor_key': 'DeltaX'}),
            '[download] Destination: a.mp4',
        ]}]})
        self.assertEqual(extractor_tiers.reached_names(text), {'Gamma', 'Delta:x', 'download'})

    def test_fixture_cases_and_reached(self):
        with tempfile.TemporaryDirectory() as root:
            site = Path(root) / 'beta'
            site.mkdir()
            (site / 'cases.json').write_text(json.dumps([{'url': 'https://b/1', 'args': ['-j']}]))
            (site / 'expected.json').write_text(json.dumps({'cases': [{'stdout': ['[Gamma] x']}]}))
            self.assertEqual(extractor_tiers.fixture_cases(Path(root)), [('beta', 'https://b/1')])
            self.assertEqual(extractor_tiers.reached(Path(root)), {'Gamma': {'beta'}})

    def classified(self):
        names, _ = extractor_tiers.parse_listing(LISTING)
        owners = {'https://a/1': 'Alpha', 'https://a/2': 'Alpha', 'https://b/1': 'Beta',
                  'https://canary/1': 'Alpha', 'https://o/1': 'generic'}
        cases = [('alpha', 'https://a/1'), ('alpha', 'https://a/2'), ('beta', 'https://b/1'),
                 ('other', 'https://o/1')]
        reached_by = {'Gamma': {'beta'}, 'Alpha': {'alpha', 'beta'}}
        canaries = [('Alpha site', 'https://canary/1')]
        return extractor_tiers.classify(names, owners, cases, reached_by, canaries)

    def test_classify(self):
        rows = {row['name']: row for row in self.classified()}
        self.assertEqual(rows['Alpha']['tier'], '1')
        self.assertEqual(rows['Alpha']['cases'], 2)
        self.assertEqual(rows['Alpha']['via'], ['beta'])
        self.assertEqual(rows['Alpha']['canary'], ['Alpha site'])
        self.assertEqual((rows['Beta']['tier'], rows['Beta']['broken']), ('2', True))
        self.assertEqual((rows['Gamma']['tier'], rows['Gamma']['cases']), ('2', 0))
        self.assertEqual(rows['html5']['tier'], '2')
        self.assertEqual(rows['generic']['tier'], '2')

    def test_untested(self):
        names = [('Lonely', False)]
        rows = extractor_tiers.classify(names, {}, [], {}, [])
        self.assertEqual(rows[0]['tier'], 'untested')

    def test_render(self):
        text = extractor_tiers.render(self.classified())
        self.assertIn('Tier 1: 1, tier 2: 4, untested: 0;', text)
        self.assertIn('| Alpha | 1 | 2 | canary: Alpha site |', text)
        self.assertIn('| Beta | 2 | 1 | marked broken upstream |', text)
        self.assertIn('| Gamma | 2 | 0 | reached via `beta` |', text)
        self.assertTrue(text.endswith('|\n'))

    def test_survey(self):
        rows = self.classified()
        upstream = [
            {'name': 'Alpha', 'working': True, 'enabled': True, 'desc': True, 'description': 'Alpha: A'},
            {'name': 'Beta', 'working': False, 'enabled': True, 'desc': True, 'description': 'Beta'},
            {'name': 'Unported', 'working': False, 'enabled': True, 'desc': True, 'description': 'Unported'},
            {'name': 'Gamma', 'working': True, 'enabled': True, 'desc': False, 'description': 'Gamma'},
            {'name': 'html5', 'working': True, 'enabled': True, 'desc': True,
             'description': 'html5: s; "h:" prefix (e.g. "hX:X")'},
            {'name': 'generic', 'working': True, 'enabled': True, 'desc': True, 'description': 'generic'},
        ]
        text = extractor_tiers.survey(rows, upstream, 'Alpha: A\nhtml5: s; "h:" prefix (e.g. "h:cat")\ngeneric\n')
        self.assertIn('Registered extractors: 5 of 6', text)
        self.assertIn('Upstream `_WORKING = False`: 2; registered here: 1: Beta', text)
        self.assertIn('Registered but not marked broken here: 0 []', text)
        self.assertIn('Registered here but absent upstream (removed): 0 []', text)
        self.assertIn('Upstream `IE_DESC = False` among ours: 1: Gamma', text)
        self.assertIn("order equals upstream's (filtered to ours): True", text)
        self.assertIn('(examples ignored): []', text)


if __name__ == '__main__':
    unittest.main()
