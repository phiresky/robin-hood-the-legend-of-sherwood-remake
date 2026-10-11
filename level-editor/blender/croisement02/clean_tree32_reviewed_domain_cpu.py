"""Bounded precision cleanup using the reviewed Tree32 source domain."""
import clean_physical_wood_precision_cpu as cleanup
from physical_tree32_reviewed_domain_cpu import reviewed_source
if __name__=='__main__':
    cleanup.source=reviewed_source
    cleanup.main()
