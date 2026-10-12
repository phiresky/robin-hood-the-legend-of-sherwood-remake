"""Review the completed saved foliage and eight verified short wood attachments."""
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
original = HERE / 'build_profile03_supports_v1.py'
source = original.read_text()
start = source.index('def main():')
end = source.index("\n\nif __name__ == '__main__':", start)
review_main = '''def main():
    check_cancel()
    section_review = build.q.OUT / 'support03-junction-sections-v1/self-review.json'
    approval = json.loads(section_review.read_text())
    construction = json.loads((build.q.OUT / 'geometry-v9-short-support03-v1/report.json').read_text())
    assert approval['model'] == construction['model']
    assert approval['status'] == 'ALL_16_SECTIONS_SELF_REVIEWED'
    original_review = HERE / 'review_support04_v1.py'
    review_source = original_review.read_text()
    review_source = review_source.replace('geometry-v8-short-support04-review-v1', 'geometry-v9-short-support03-review-v1')
    review_source = review_source.replace('geometry-v8-short-support04-v3', 'geometry-v9-short-support03-v1')
    review_source = review_source.replace("        active=[o for o in meshes", "        check_cancel()\\n        active=[o for o in meshes")
    namespace = dict(__name__='saved_profile03_review', __file__=str(original_review), check_cancel=check_cancel)
    exec(compile(review_source, str(original_review), 'exec'), namespace)
    namespace['main']()
'''
source = source[:start] + review_main + source[end:]
source = source.replace('cancel-profile03-support-build-v1', 'cancel-profile03-support-review-v1')
exec(compile(source, str(original), 'exec'), {'__name__':'__main__', '__file__':str(original)})
