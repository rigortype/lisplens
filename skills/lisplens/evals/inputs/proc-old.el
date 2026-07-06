;;; proc.el --- -*- lexical-binding: t; -*-
(defun proc-run (items)
  "Process ITEMS."
  (when (proc-ready-p)
    (dolist (it items)
      (proc-handle it))))
