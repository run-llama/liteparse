"""Check leakage reservation and conservative low-information family matching."""
import unittest

import family_audit as audit


class FamilyAuditTests(unittest.TestCase):
    def test_transitive_family_reserves_old_siblings(self):
        groups = audit.components(['a', 'b', 'c', 'd'], [
            {'left': 'a', 'right': 'b'}, {'left': 'b', 'right': 'c'},
        ], {'c'})
        reserved = next(group for group in groups if group['new_members'])
        self.assertEqual(reserved['members'], ['a', 'b', 'c'])
        self.assertEqual(reserved['reservation'], 'holdout_candidate_entire_component')
        self.assertEqual(next(group for group in groups if group['members'] == ['d'])['reservation'], 'existing_only')

    def test_large_exact_body_group_keeps_transitive_links(self):
        body = ' '.join('word' + str(i) for i in range(30))
        base = {**audit.body_features(body), **audit.paragraph_features(None)}
        rows = [{**base, 'source_sha256': str(i)} for i in range(130)]
        pairs, _ = audit.candidate_pairs(rows)
        edges = [audit.edge(rows[i], rows[j]) for i, j in pairs]
        groups = audit.components([row['source_sha256'] for row in rows], edges, {'129'})
        self.assertEqual(len(groups), 1)
        self.assertEqual(len(groups[0]['members']), 130)

    def test_common_empty_formatting_does_not_link_documents(self):
        a = {**audit.body_features('One\r'), **audit.paragraph_features(None), 'source_sha256': 'a'}
        b = {**audit.body_features('Two\r'), **audit.paragraph_features(None), 'source_sha256': 'b'}
        self.assertIsNone(audit.edge(a, b))
        self.assertEqual(audit.overlap([], []), 0)

    def test_styles_are_canonicalized_and_piece_fragments_collapsed(self):
        first = {'spans': [
            {'style_index': 7, 'raw_modifiers': [1, 2]},
            {'style_index': 7, 'raw_modifiers': [1, 2]},
            {'style_index': 8, 'raw_modifiers': [3, 4]},
            {'style_index': 7, 'raw_modifiers': [1, 2]},
        ]}
        second = {'spans': [
            {'style_index': 33, 'raw_modifiers': [1, 2]},
            {'style_index': 44, 'raw_modifiers': [3, 4]},
            {'style_index': 33, 'raw_modifiers': [1, 2]},
        ]}
        self.assertEqual(audit.paragraph_features(first)['papx_sequence_sha256'],
                         audit.paragraph_features(second)['papx_sequence_sha256'])

    def test_identical_content_links_without_resolved_styles(self):
        body = ' '.join('word' + str(i) for i in range(30))
        a = {**audit.body_features(body), **audit.paragraph_features(None), 'source_sha256': 'a'}
        b = {**audit.body_features(body), **audit.paragraph_features(None), 'source_sha256': 'b'}
        self.assertIn('exact_body', audit.edge(a, b)['reasons'])


if __name__ == '__main__':
    unittest.main()
