Empty on purpose. `ansible.cfg` points the collections path here, so the roles can use nothing
but `ansible.builtin`, even on a system that has collections installed.
