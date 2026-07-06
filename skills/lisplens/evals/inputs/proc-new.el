;;; proc.el --- -*- lexical-binding: t; -*-
(defun proc-run (items &optional force)
  "Process ITEMS."
  (unless (and (not force) (proc-busy-p))
    (dolist (it items)
      (proc-handle it)
      (proc-log it))))
