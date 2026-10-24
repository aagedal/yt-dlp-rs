import unittest

from generate_adobepass_mso_info import mso_info, render

SOURCE = '''
MSO_INFO = {
    'A_1': {
        'name': 'Provider "A" \\\\ One',
        'username_field': 'user',
        'password_field': 'passwd',
        'login_hostname': 'login.example.com',
    },
    'b': {
        'name': 'B',
    },
}
'''


class GenerateAdobePassMsoInfoTest(unittest.TestCase):
    def test_entries_render_in_upstream_order_with_rust_escapes(self):
        text = render(mso_info(SOURCE))
        self.assertIn(
            '    (\n        "A_1",\n        MsoInfo {\n'
            '            name: "Provider \\"A\\" \\\\ One",\n'
            '            username_field: Some("user"),\n'
            '            password_field: Some("passwd"),\n'
            '            login_hostname: Some("login.example.com"),\n'
            '        },\n    ),\n', text)
        self.assertLess(text.index('"A_1"'), text.index('"b"'))
        self.assertIn('            username_field: None,\n', text)

    def test_unknown_keys_are_rejected(self):
        with self.assertRaises(SystemExit):
            render({'x': {'name': 'X', 'other': 'y'}})


if __name__ == '__main__':
    unittest.main()
