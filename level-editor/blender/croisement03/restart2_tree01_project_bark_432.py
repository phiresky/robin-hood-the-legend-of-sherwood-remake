"""Project safe retained bark plus the separately reviewed stem interior proposal."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_tree01_project_bark_seeds as project


if __name__ == '__main__':
    project.EXTRA_PROOF = project.B / 'tree01-native-transition-saved-v2/bark-receiver-147-v1/receipt.json'
    assert json.loads(project.EXTRA_PROOF.read_text())['status'] == 'PASS_SAVED_BARK_RECEIVERS'
    project.OUT = project.B / 'tree01-bark-source-432-projection-v1'
    project.main()
